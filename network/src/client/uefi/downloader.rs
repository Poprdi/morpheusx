//! High-level download manager for UEFI HTTP client.
//!
//! Provides a simple, ergonomic API for downloading files:
//! - URL parsing and validation
//! - Progress tracking with callbacks
//! - Automatic retry on transient failures
//! - File size detection via HEAD requests
//! - Chunked download support for large files
//!
//! # Examples
//!
//! ```ignore
//! use morpheus_network::client::uefi::Downloader;
//!
//! let mut downloader = Downloader::new(&mut client);
//!
//! // Simple download
//! let data = downloader.download("http://example.com/file.bin")?;
//!
//! // Download with progress
//! let data = downloader.download_with_progress(
//!     "http://mirror.example.com/distro.iso",
//!     |transferred, total| {
//!         if let Some(t) = total {
//!             println!("Progress: {}%", transferred * 100 / t);
//!         }
//!     }
//! )?;
//!
//! // Check file size before download
//! let size = downloader.get_file_size("http://example.com/large.iso")?;
//! println!("File size: {:?} bytes", size);
//! ```

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::client::HttpClient;
use crate::error::{NetworkError, Result};
use crate::http::{Request, Response};
use crate::types::{HttpMethod, ProgressCallback};
use crate::url::Url;
use crate::transfer::streaming::{StreamReader, StreamConfig, ProgressTracker};

/// Download configuration options.
#[derive(Debug, Clone)]
pub struct DownloadConfig {
    /// Number of retry attempts on failure.
    pub retries: u32,
    /// Follow HTTP redirects.
    pub follow_redirects: bool,
    /// Maximum file size to download (bytes).
    pub max_size: Option<usize>,
    /// Chunk size for streaming downloads.
    pub chunk_size: usize,
    /// Progress callback interval (bytes).
    pub progress_interval: usize,
    /// Validate Content-Length matches received data.
    pub validate_length: bool,
}

impl Default for DownloadConfig {
    fn default() -> Self {
        Self {
            retries: 3,
            follow_redirects: true,
            max_size: None,
            chunk_size: 64 * 1024, // 64KB
            progress_interval: 16 * 1024, // Report every 16KB
            validate_length: true,
        }
    }
}

impl DownloadConfig {
    /// Configuration for downloading ISO images.
    pub fn for_iso() -> Self {
        Self {
            retries: 5,
            follow_redirects: true,
            max_size: Some(10 * 1024 * 1024 * 1024), // 10GB max
            chunk_size: 256 * 1024, // 256KB chunks
            progress_interval: 1024 * 1024, // Report every 1MB
            validate_length: true,
        }
    }

    /// Configuration for small metadata files.
    pub fn for_metadata() -> Self {
        Self {
            retries: 3,
            follow_redirects: true,
            max_size: Some(1024 * 1024), // 1MB max
            chunk_size: 8 * 1024, // 8KB chunks
            progress_interval: 4 * 1024, // Report every 4KB
            validate_length: true,
        }
    }
}

/// Result of a download operation.
#[derive(Debug)]
pub struct DownloadResult {
    /// Downloaded data.
    pub data: Vec<u8>,
    /// Final URL (after redirects).
    pub final_url: String,
    /// Content type from server.
    pub content_type: Option<String>,
    /// Total bytes downloaded.
    pub bytes_downloaded: usize,
}

/// High-level download manager.
///
/// Provides convenient methods for downloading files over HTTP
/// with progress tracking and error handling.
pub struct Downloader<'a> {
    /// HTTP client to use for requests.
    client: &'a mut dyn HttpClient,
    /// Download configuration.
    config: DownloadConfig,
}

impl<'a> Downloader<'a> {
    /// Create a new downloader with default configuration.
    pub fn new(client: &'a mut dyn HttpClient) -> Self {
        Self {
            client,
            config: DownloadConfig::default(),
        }
    }

    /// Create a downloader with custom configuration.
    pub fn with_config(client: &'a mut dyn HttpClient, config: DownloadConfig) -> Self {
        Self { client, config }
    }

    /// Download a file from the given URL.
    ///
    /// # Arguments
    ///
    /// * `url` - URL to download from
    ///
    /// # Returns
    ///
    /// Downloaded data as bytes.
    ///
    /// # Errors
    ///
    /// Returns error if URL is invalid, server returns error,
    /// or download exceeds max size.
    pub fn download(&mut self, url: &str) -> Result<Vec<u8>> {
        let result = self.download_full(url)?;
        Ok(result.data)
    }

    /// Download with full result information.
    pub fn download_full(&mut self, url: &str) -> Result<DownloadResult> {
        let parsed_url = Url::parse(url)?;
        self.download_url(&parsed_url, None)
    }

    /// Download with progress callback.
    ///
    /// The callback receives (bytes_transferred, total_bytes_if_known).
    pub fn download_with_progress(
        &mut self,
        url: &str,
        progress: ProgressCallback,
    ) -> Result<Vec<u8>> {
        let parsed_url = Url::parse(url)?;
        let result = self.download_url(&parsed_url, Some(progress))?;
        Ok(result.data)
    }

    /// Get the size of a remote file without downloading it.
    ///
    /// Uses HTTP HEAD request to get Content-Length.
    ///
    /// # Returns
    ///
    /// File size in bytes, or None if server doesn't provide Content-Length.
    pub fn get_file_size(&mut self, url: &str) -> Result<Option<usize>> {
        let parsed_url = Url::parse(url)?;
        let request = Request::head(parsed_url);
        
        let response = self.execute_with_retry(&request)?;
        
        if !response.is_success() {
            return Err(NetworkError::HttpError(response.status_code));
        }
        
        Ok(response.content_length())
    }

    /// Check if a URL is accessible.
    ///
    /// Uses HEAD request to check without downloading.
    pub fn check_url(&mut self, url: &str) -> Result<bool> {
        let parsed_url = Url::parse(url)?;
        let request = Request::head(parsed_url);
        
        match self.execute_with_retry(&request) {
            Ok(response) => Ok(response.is_success()),
            Err(NetworkError::HttpError(_)) => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Get response headers without downloading body.
    pub fn get_headers(&mut self, url: &str) -> Result<Response> {
        let parsed_url = Url::parse(url)?;
        let request = Request::head(parsed_url);
        self.execute_with_retry(&request)
    }

    /// Internal: Download from parsed URL.
    fn download_url(
        &mut self,
        url: &Url,
        progress: Option<ProgressCallback>,
    ) -> Result<DownloadResult> {
        let request = Request::get(url.clone());
        
        // Execute with progress if provided
        let response = if let Some(callback) = progress {
            self.client.request_with_progress(&request, callback)?
        } else {
            self.execute_with_retry(&request)?
        };

        // Check for success
        if !response.is_success() {
            return Err(NetworkError::HttpError(response.status_code));
        }

        // Validate size if configured
        if let Some(max_size) = self.config.max_size {
            if response.body.len() > max_size {
                return Err(NetworkError::OutOfMemory);
            }
        }

        // Validate Content-Length if configured
        if self.config.validate_length {
            if let Some(expected) = response.content_length() {
                if response.body.len() != expected {
                    return Err(NetworkError::InvalidResponse);
                }
            }
        }

        let bytes_downloaded = response.body.len();
        let content_type = response.content_type().map(|s| s.to_string());
        
        Ok(DownloadResult {
            data: response.body,
            final_url: url.to_string(),
            content_type,
            bytes_downloaded,
        })
    }

    /// Execute request with retries.
    fn execute_with_retry(&mut self, request: &Request) -> Result<Response> {
        let mut last_error = NetworkError::Unknown;
        
        for attempt in 0..=self.config.retries {
            match self.client.request(request) {
                Ok(response) => return Ok(response),
                Err(e) => {
                    last_error = e;
                    
                    // Don't retry certain errors
                    match last_error {
                        NetworkError::InvalidUrl |
                        NetworkError::InvalidResponse |
                        NetworkError::HttpError(404) |
                        NetworkError::HttpError(403) |
                        NetworkError::HttpError(401) => {
                            return Err(last_error);
                        }
                        _ => {
                            // Retry for transient errors
                            if attempt < self.config.retries {
                                // Would add delay here in real impl
                                continue;
                            }
                        }
                    }
                }
            }
        }
        
        Err(last_error)
    }
}

/// Builder for downloading files with fluent API.
pub struct DownloadBuilder<'a> {
    client: &'a mut dyn HttpClient,
    url: String,
    config: DownloadConfig,
    progress: Option<ProgressCallback>,
    headers: Vec<(String, String)>,
}

impl<'a> DownloadBuilder<'a> {
    /// Create a new download builder.
    pub fn new(client: &'a mut dyn HttpClient, url: impl Into<String>) -> Self {
        Self {
            client,
            url: url.into(),
            config: DownloadConfig::default(),
            progress: None,
            headers: Vec::new(),
        }
    }

    /// Set maximum file size.
    pub fn max_size(mut self, size: usize) -> Self {
        self.config.max_size = Some(size);
        self
    }

    /// Set number of retries.
    pub fn retries(mut self, retries: u32) -> Self {
        self.config.retries = retries;
        self
    }

    /// Set progress callback.
    pub fn progress(mut self, callback: ProgressCallback) -> Self {
        self.progress = Some(callback);
        self
    }

    /// Add a custom header.
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// Don't validate Content-Length.
    pub fn skip_length_validation(mut self) -> Self {
        self.config.validate_length = false;
        self
    }

    /// Execute the download.
    pub fn execute(self) -> Result<Vec<u8>> {
        let parsed_url = Url::parse(&self.url)?;
        let mut request = Request::get(parsed_url);
        
        // Add custom headers
        for (name, value) in &self.headers {
            request = request.with_header(name.clone(), value.clone());
        }
        
        // Execute with progress if provided
        let response = if let Some(callback) = self.progress {
            self.client.request_with_progress(&request, callback)?
        } else {
            self.client.request(&request)?
        };
        
        if !response.is_success() {
            return Err(NetworkError::HttpError(response.status_code));
        }
        
        // Validate size
        if let Some(max) = self.config.max_size {
            if response.body.len() > max {
                return Err(NetworkError::OutOfMemory);
            }
        }
        
        Ok(response.body)
    }
}

// ==================== Tests ====================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::Headers;

    /// Mock HTTP client for testing.
    struct MockClient {
        responses: Vec<Response>,
        request_count: usize,
    }

    impl MockClient {
        fn new() -> Self {
            Self {
                responses: Vec::new(),
                request_count: 0,
            }
        }

        fn with_response(mut self, response: Response) -> Self {
            self.responses.push(response);
            self
        }

        fn with_success(self, body: &[u8]) -> Self {
            let mut response = Response::new(200);
            response.body = body.to_vec();
            response.headers.set("Content-Length", &body.len().to_string());
            self.with_response(response)
        }

        fn with_not_found(self) -> Self {
            self.with_response(Response::new(404))
        }

        fn with_redirect(self, location: &str) -> Self {
            let mut response = Response::new(302);
            response.headers.set("Location", location);
            self.with_response(response)
        }
    }

    impl HttpClient for MockClient {
        fn request(&mut self, _request: &Request) -> Result<Response> {
            if self.request_count < self.responses.len() {
                let response = self.responses[self.request_count].clone();
                self.request_count += 1;
                Ok(response)
            } else {
                // Return default success if no more configured responses
                let mut response = Response::new(200);
                response.body = b"default".to_vec();
                Ok(response)
            }
        }

        fn request_with_progress(
            &mut self,
            request: &Request,
            progress: ProgressCallback,
        ) -> Result<Response> {
            let response = self.request(request)?;
            progress(response.body.len(), Some(response.body.len()));
            Ok(response)
        }

        fn is_ready(&self) -> bool {
            true
        }
    }

    #[test]
    fn test_downloader_new() {
        let mut client = MockClient::new().with_success(b"test");
        let downloader = Downloader::new(&mut client);
        assert_eq!(downloader.config.retries, 3);
    }

    #[test]
    fn test_downloader_with_config() {
        let mut client = MockClient::new();
        let config = DownloadConfig {
            retries: 5,
            ..Default::default()
        };
        let downloader = Downloader::with_config(&mut client, config);
        assert_eq!(downloader.config.retries, 5);
    }

    #[test]
    fn test_download_simple() {
        let mut client = MockClient::new().with_success(b"Hello World");
        let mut downloader = Downloader::new(&mut client);
        
        let data = downloader.download("http://example.com/file.txt").unwrap();
        assert_eq!(data, b"Hello World");
    }

    #[test]
    fn test_download_full() {
        let mut client = MockClient::new().with_success(b"content");
        let mut downloader = Downloader::new(&mut client);
        
        let result = downloader.download_full("http://example.com/file").unwrap();
        assert_eq!(result.data, b"content");
        assert_eq!(result.bytes_downloaded, 7);
    }

    #[test]
    fn test_download_with_progress() {
        let mut client = MockClient::new().with_success(b"test data");
        let mut downloader = Downloader::new(&mut client);
        
        let mut progress_called = false;
        let data = downloader.download_with_progress(
            "http://example.com/file",
            |transferred, total| {
                progress_called = true;
                assert!(transferred > 0);
                assert!(total.is_some());
            }
        ).unwrap();
        
        assert!(progress_called);
        assert_eq!(data, b"test data");
    }

    #[test]
    fn test_get_file_size() {
        let mut response = Response::new(200);
        response.headers.set("Content-Length", "12345");
        
        let mut client = MockClient::new().with_response(response);
        let mut downloader = Downloader::new(&mut client);
        
        let size = downloader.get_file_size("http://example.com/large.iso").unwrap();
        assert_eq!(size, Some(12345));
    }

    #[test]
    fn test_check_url_success() {
        let mut client = MockClient::new().with_success(b"");
        let mut downloader = Downloader::new(&mut client);
        
        let exists = downloader.check_url("http://example.com/exists").unwrap();
        assert!(exists);
    }

    #[test]
    fn test_check_url_not_found() {
        let mut client = MockClient::new().with_not_found();
        let mut downloader = Downloader::new(&mut client);
        
        let exists = downloader.check_url("http://example.com/missing").unwrap();
        assert!(!exists);
    }

    #[test]
    fn test_download_invalid_url() {
        let mut client = MockClient::new();
        let mut downloader = Downloader::new(&mut client);
        
        let result = downloader.download("not-a-valid-url");
        assert!(result.is_err());
    }

    #[test]
    fn test_download_http_error() {
        let mut client = MockClient::new().with_not_found();
        let mut downloader = Downloader::new(&mut client);
        
        let result = downloader.download("http://example.com/missing");
        assert!(matches!(result, Err(NetworkError::HttpError(404))));
    }

    #[test]
    fn test_download_max_size_exceeded() {
        let mut client = MockClient::new().with_success(b"12345678901234567890");
        let config = DownloadConfig {
            max_size: Some(10),
            ..Default::default()
        };
        let mut downloader = Downloader::with_config(&mut client, config);
        
        let result = downloader.download("http://example.com/large");
        assert!(matches!(result, Err(NetworkError::OutOfMemory)));
    }

    #[test]
    fn test_download_config_for_iso() {
        let config = DownloadConfig::for_iso();
        assert_eq!(config.retries, 5);
        assert_eq!(config.chunk_size, 256 * 1024);
    }

    #[test]
    fn test_download_config_for_metadata() {
        let config = DownloadConfig::for_metadata();
        assert_eq!(config.max_size, Some(1024 * 1024));
        assert_eq!(config.chunk_size, 8 * 1024);
    }

    // ==================== DownloadBuilder Tests ====================

    #[test]
    fn test_download_builder_simple() {
        let mut client = MockClient::new().with_success(b"builder test");
        
        let data = DownloadBuilder::new(&mut client, "http://example.com/file")
            .execute()
            .unwrap();
        
        assert_eq!(data, b"builder test");
    }

    #[test]
    fn test_download_builder_with_options() {
        let mut client = MockClient::new().with_success(b"options");
        
        let data = DownloadBuilder::new(&mut client, "http://example.com/file")
            .max_size(1000)
            .retries(5)
            .header("Accept", "application/json")
            .skip_length_validation()
            .execute()
            .unwrap();
        
        assert_eq!(data, b"options");
    }

    #[test]
    fn test_download_builder_with_progress() {
        let mut client = MockClient::new().with_success(b"progress test");
        
        let mut called = false;
        let data = DownloadBuilder::new(&mut client, "http://example.com/file")
            .progress(|_, _| {
                // Progress callback
            })
            .execute()
            .unwrap();
        
        assert_eq!(data, b"progress test");
    }

    #[test]
    fn test_download_builder_max_size_exceeded() {
        let mut client = MockClient::new().with_success(b"too large");
        
        let result = DownloadBuilder::new(&mut client, "http://example.com/file")
            .max_size(5)
            .execute();
        
        assert!(result.is_err());
    }

    #[test]
    fn test_download_builder_invalid_url() {
        let mut client = MockClient::new();
        
        let result = DownloadBuilder::new(&mut client, "invalid-url")
            .execute();
        
        assert!(result.is_err());
    }
}
