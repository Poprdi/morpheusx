//! UEFI HTTP client implementation.
//!
//! Provides HTTP client functionality using UEFI HTTP protocols.
//! This is the main interface for making HTTP requests in UEFI environment.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────┐
//! │                  UefiHttpClient                         │
//! ├─────────────────────────────────────────────────────────┤
//! │  ┌─────────────┐  ┌──────────────┐  ┌───────────────┐  │
//! │  │ Request     │  │ Protocol     │  │ Response      │  │
//! │  │ Builder     │──│ Manager      │──│ Parser        │  │
//! │  └─────────────┘  └──────────────┘  └───────────────┘  │
//! │         │                │                  │          │
//! │         ▼                ▼                  ▼          │
//! │  ┌─────────────────────────────────────────────────┐   │
//! │  │                UEFI HTTP Protocol               │   │
//! │  └─────────────────────────────────────────────────┘   │
//! └─────────────────────────────────────────────────────────┘
//! ```
//!
//! # Usage
//!
//! ```ignore
//! use morpheus_network::client::uefi::UefiHttpClient;
//!
//! let mut client = UefiHttpClient::new(boot_services)?;
//! let response = client.get("http://example.com/file.iso")?;
//! ```

use alloc::vec::Vec;
use alloc::vec;
use alloc::string::{String, ToString};
use core::ptr;

use crate::client::HttpClient;
use crate::error::{NetworkError, Result};
use crate::http::{Request, Response, Headers};
use crate::protocol::uefi::ProtocolManager;
use crate::protocol::uefi::bindings::{
    HttpProtocol, HttpMessage, HttpMessageData, HttpRequestData, HttpResponseData,
    HttpToken, HttpHeader, HttpMethod, HttpStatusCode, Event,
    status,
};
use crate::types::{HttpMethod as TypesHttpMethod, ProgressCallback};
use crate::url::Url;
use crate::utils::string::{ascii_to_utf16, utf16_to_ascii};
use crate::transfer::streaming::{StreamReader, StreamConfig};

/// Configuration for the UEFI HTTP client.
#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// Maximum response body size to accept.
    pub max_response_size: usize,
    /// Timeout for individual operations in milliseconds.
    pub timeout_ms: u32,
    /// Number of retries on transient failures.
    pub retries: u32,
    /// Follow redirects automatically.
    pub follow_redirects: bool,
    /// Maximum number of redirects to follow.
    pub max_redirects: u32,
    /// Buffer size for streaming.
    pub buffer_size: usize,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            max_response_size: 100 * 1024 * 1024, // 100MB
            timeout_ms: 30_000,                    // 30 seconds
            retries: 3,
            follow_redirects: true,
            max_redirects: 10,
            buffer_size: 64 * 1024, // 64KB
        }
    }
}

impl ClientConfig {
    /// Create config for downloading large files.
    pub fn for_large_downloads() -> Self {
        Self {
            max_response_size: 10 * 1024 * 1024 * 1024, // 10GB
            timeout_ms: 60_000,                          // 60 seconds
            retries: 5,
            follow_redirects: true,
            max_redirects: 10,
            buffer_size: 256 * 1024, // 256KB
        }
    }
}

/// UEFI HTTP client.
///
/// Provides HTTP request capabilities using UEFI protocols.
pub struct UefiHttpClient {
    /// Protocol manager for UEFI operations.
    protocol_manager: ProtocolManager,
    /// Client configuration.
    config: ClientConfig,
    /// Whether the client is initialized.
    initialized: bool,
    /// Reusable buffer for URL conversion.
    url_buffer: Vec<u16>,
    /// Reusable buffer for headers.
    header_buffer: Vec<HttpHeader>,
    /// Storage for header strings (must outlive headers).
    header_strings: Vec<(Vec<u8>, Vec<u8>)>,
}

impl UefiHttpClient {
    /// Create a new UEFI HTTP client.
    ///
    /// Note: In actual UEFI environment, this would take boot_services.
    /// For now, creates an uninitialized client.
    pub fn new() -> Result<Self> {
        Ok(Self {
            protocol_manager: ProtocolManager::new(),
            config: ClientConfig::default(),
            initialized: false,
            url_buffer: Vec::new(),
            header_buffer: Vec::new(),
            header_strings: Vec::new(),
        })
    }

    /// Create with custom configuration.
    pub fn with_config(config: ClientConfig) -> Result<Self> {
        Ok(Self {
            protocol_manager: ProtocolManager::new(),
            config,
            initialized: false,
            url_buffer: Vec::new(),
            header_buffer: Vec::new(),
            header_strings: Vec::new(),
        })
    }

    /// Initialize the client directly from UEFI BootServices pointer.
    ///
    /// This is the simplest initialization path for bootloader integration.
    /// Locates HTTP protocols via BootServices and configures the client.
    ///
    /// # Arguments
    ///
    /// * `boot_services` - Pointer to UEFI Boot Services table
    ///
    /// # Safety
    ///
    /// - `boot_services` must be a valid pointer to UEFI Boot Services.
    /// - Must be called before `ExitBootServices()`.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let bs = unsafe { &*system_table.boot_services };
    /// let mut client = UefiHttpClient::new()?;
    /// unsafe { client.initialize(bs)?; }
    /// ```
    #[cfg(target_os = "uefi")]
    pub unsafe fn initialize(
        &mut self,
        boot_services: &crate::protocol::uefi::bindings::BootServices,
    ) -> Result<()> {
        self.protocol_manager.initialize_from_boot_services(boot_services as *const _)?;
        self.initialized = true;
        Ok(())
    }

    /// Initialize the client with UEFI protocols via closures.
    ///
    /// This is the original initialization path using callback closures.
    /// For most cases, prefer `initialize()` with direct BootServices pointer.
    ///
    /// # Safety
    ///
    /// This method interacts with UEFI boot services.
    /// Must be called before making requests.
    #[cfg(target_os = "uefi")]
    pub unsafe fn initialize_with_closures<F, G>(
        &mut self,
        locate_protocol: F,
        open_protocol: G,
    ) -> Result<()>
    where
        F: Fn(&crate::protocol::uefi::bindings::Guid) -> Option<*mut crate::protocol::uefi::bindings::ServiceBindingProtocol>,
        G: Fn(crate::protocol::uefi::bindings::Handle, &crate::protocol::uefi::bindings::Guid) -> Option<*mut HttpProtocol>,
    {
        self.protocol_manager.initialize(locate_protocol, open_protocol)?;
        self.initialized = true;
        Ok(())
    }

    /// Initialize with mock for testing.
    #[cfg(test)]
    pub fn initialize_mock(&mut self) -> Result<()> {
        self.protocol_manager.initialize_mock()?;
        self.initialized = true;
        Ok(())
    }

    /// Shutdown the client.
    ///
    /// # Safety
    ///
    /// Releases UEFI resources.
    pub unsafe fn shutdown(&mut self) -> Result<()> {
        self.protocol_manager.shutdown()?;
        self.initialized = false;
        Ok(())
    }

    /// Perform a GET request.
    pub fn get(&mut self, url: &str) -> Result<Response> {
        let parsed_url = Url::parse(url)?;
        let request = Request::get(parsed_url);
        self.request(&request)
    }

    /// Perform a HEAD request (get headers only).
    pub fn head(&mut self, url: &str) -> Result<Response> {
        let parsed_url = Url::parse(url)?;
        let request = Request::head(parsed_url);
        self.request(&request)
    }

    /// Perform a POST request.
    pub fn post(&mut self, url: &str, body: Vec<u8>) -> Result<Response> {
        let parsed_url = Url::parse(url)?;
        let request = Request::post(parsed_url).with_body(body);
        self.request(&request)
    }

    /// Execute request with redirect handling.
    fn execute_with_redirects(&mut self, request: &Request) -> Result<Response> {
        let mut current_request = request.clone();
        let mut redirect_count = 0;

        loop {
            let response = self.execute_single_request(&current_request)?;

            if !self.config.follow_redirects || !response.is_redirect() {
                return Ok(response);
            }

            redirect_count += 1;
            if redirect_count > self.config.max_redirects {
                return Err(NetworkError::HttpError(response.status_code));
            }

            // Get redirect location
            let location = response.location().ok_or(NetworkError::InvalidResponse)?;
            let new_url = Url::parse(location)?;
            
            // Create new request for redirect
            current_request = Request::get(new_url);
        }
    }

    /// Execute a single HTTP request (no redirect handling).
    fn execute_single_request(&mut self, request: &Request) -> Result<Response> {
        // For non-UEFI builds (testing), return mock response
        #[cfg(not(target_os = "uefi"))]
        {
            return self.mock_request(request);
        }

        #[cfg(target_os = "uefi")]
        {
            self.execute_uefi_request(request)
        }
    }

    /// Execute request using UEFI HTTP protocol.
    #[cfg(target_os = "uefi")]
    fn execute_uefi_request(&mut self, request: &Request) -> Result<Response> {
        if !self.initialized {
            return Err(NetworkError::InitializationFailed);
        }

        let http = self.protocol_manager.http_protocol()
            .ok_or(NetworkError::ProtocolNotAvailable)?;

        // Build UEFI request structures
        self.prepare_request_buffers(request)?;

        // Create request data
        let mut request_data = HttpRequestData {
            method: HttpMethod::from_types_method(request.method),
            url: self.url_buffer.as_ptr(),
        };

        // Create message
        let mut message = HttpMessage {
            data: HttpMessageData { request: &mut request_data },
            header_count: self.header_buffer.len(),
            headers: if self.header_buffer.is_empty() {
                ptr::null_mut()
            } else {
                self.header_buffer.as_mut_ptr()
            },
            body_length: request.body.as_ref().map(|b| b.len()).unwrap_or(0),
            body: request.body.as_ref()
                .map(|b| b.as_ptr() as *mut u8)
                .unwrap_or(ptr::null_mut()),
        };

        // Create token
        let mut token = HttpToken {
            event: ptr::null_mut(),
            status: 0,
            message: &mut message,
        };

        // Send request
        unsafe {
            let status = ((*http).request)(http, &mut token);
            if !status::is_success(status) {
                return Err(NetworkError::ConnectionFailed);
            }

            // Poll for completion
            self.poll_completion(http)?;
        }

        // Get response
        self.receive_response(http)
    }

    /// Prepare buffers for UEFI request.
    fn prepare_request_buffers(&mut self, request: &Request) -> Result<()> {
        // Convert URL to UTF-16
        let url_str = request.url.to_string();
        self.url_buffer = ascii_to_utf16(&url_str);

        // Convert headers
        self.header_strings.clear();
        self.header_buffer.clear();

        for header in request.headers.iter() {
            let name_bytes: Vec<u8> = header.name.bytes().chain(core::iter::once(0)).collect();
            let value_bytes: Vec<u8> = header.value.bytes().chain(core::iter::once(0)).collect();
            self.header_strings.push((name_bytes, value_bytes));
        }

        for (name, value) in &self.header_strings {
            self.header_buffer.push(HttpHeader {
                field_name: name.as_ptr(),
                field_value: value.as_ptr(),
            });
        }

        Ok(())
    }

    /// Poll for request/response completion.
    #[cfg(target_os = "uefi")]
    unsafe fn poll_completion(&self, http: *mut HttpProtocol) -> Result<()> {
        // In a real implementation, this would use events or polling
        // For simplicity, we just poll in a loop
        let mut attempts = 0;
        let max_attempts = self.config.timeout_ms / 10;

        loop {
            let status = ((*http).poll)(http);
            if status::is_success(status) {
                return Ok(());
            }
            
            attempts += 1;
            if attempts > max_attempts {
                return Err(NetworkError::Timeout);
            }
            
            // Small delay would go here in real implementation
        }
    }

    /// Receive HTTP response.
    #[cfg(target_os = "uefi")]
    fn receive_response(&mut self, http: *mut HttpProtocol) -> Result<Response> {
        // Allocate response buffer
        let mut body_buffer = vec![0u8; self.config.buffer_size];
        
        let mut response_data = HttpResponseData {
            status_code: HttpStatusCode(0),
        };

        let mut message = HttpMessage {
            data: HttpMessageData { response: &mut response_data },
            header_count: 0,
            headers: ptr::null_mut(),
            body_length: body_buffer.len(),
            body: body_buffer.as_mut_ptr(),
        };

        let mut token = HttpToken {
            event: ptr::null_mut(),
            status: 0,
            message: &mut message,
        };

        // Request response
        unsafe {
            let status = ((*http).response)(http, &mut token);
            if !status::is_success(status) {
                return Err(NetworkError::InvalidResponse);
            }

            // Poll for completion
            self.poll_completion(http)?;
        }

        // Build response
        let status_code = unsafe { (*message.data.response).status_code.code() as u16 };
        let body_len = message.body_length;
        body_buffer.truncate(body_len);

        let mut response = Response::new(status_code);
        response.body = body_buffer;
        
        // Parse headers from token
        // (In real impl, would parse from message.headers)
        
        Ok(response)
    }

    /// Mock request for testing.
    #[cfg(not(target_os = "uefi"))]
    fn mock_request(&self, request: &Request) -> Result<Response> {
        // Return mock response for testing
        let mut response = Response::new(200);
        response.headers.set("Content-Type", "text/plain");
        response.headers.set("Content-Length", "5");
        response.body = b"Hello".to_vec();
        Ok(response)
    }
}

impl Default for UefiHttpClient {
    fn default() -> Self {
        Self::new().expect("Failed to create default client")
    }
}

impl HttpClient for UefiHttpClient {
    fn request(&mut self, request: &Request) -> Result<Response> {
        self.execute_with_redirects(request)
    }

    fn request_with_progress(
        &mut self,
        request: &Request,
        progress: ProgressCallback,
    ) -> Result<Response> {
        // For progress tracking, we'd use streaming
        // For now, just call regular request and report at end
        let response = self.request(request)?;
        progress(response.body.len(), Some(response.body.len()));
        Ok(response)
    }

    fn is_ready(&self) -> bool {
        self.initialized && self.protocol_manager.is_ready()
    }
}

// ==================== Tests ====================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_new() {
        let client = UefiHttpClient::new().unwrap();
        assert!(!client.is_ready());
    }

    #[test]
    fn test_client_with_config() {
        let config = ClientConfig {
            timeout_ms: 5000,
            retries: 1,
            ..Default::default()
        };
        let client = UefiHttpClient::with_config(config).unwrap();
        assert!(!client.is_ready());
    }

    #[test]
    fn test_client_config_default() {
        let config = ClientConfig::default();
        assert_eq!(config.timeout_ms, 30_000);
        assert_eq!(config.retries, 3);
        assert!(config.follow_redirects);
    }

    #[test]
    fn test_client_config_large_downloads() {
        let config = ClientConfig::for_large_downloads();
        assert_eq!(config.timeout_ms, 60_000);
        assert_eq!(config.buffer_size, 256 * 1024);
    }

    #[test]
    fn test_client_mock_initialize() {
        let mut client = UefiHttpClient::new().unwrap();
        client.initialize_mock().unwrap();
        assert!(client.is_ready());
    }

    #[test]
    fn test_client_get_mock() {
        let mut client = UefiHttpClient::new().unwrap();
        client.initialize_mock().unwrap();
        
        let response = client.get("http://example.com/test").unwrap();
        assert_eq!(response.status_code, 200);
        assert_eq!(response.body, b"Hello");
    }

    #[test]
    fn test_client_head_mock() {
        let mut client = UefiHttpClient::new().unwrap();
        client.initialize_mock().unwrap();
        
        let response = client.head("http://example.com/test").unwrap();
        assert!(response.is_success());
    }

    #[test]
    fn test_client_post_mock() {
        let mut client = UefiHttpClient::new().unwrap();
        client.initialize_mock().unwrap();
        
        let body = b"test data".to_vec();
        let response = client.post("http://example.com/api", body).unwrap();
        assert!(response.is_success());
    }

    #[test]
    fn test_prepare_request_buffers() {
        let mut client = UefiHttpClient::new().unwrap();
        
        let url = Url::parse("http://example.com/path").unwrap();
        let request = Request::get(url).with_header("Accept", "text/html");
        
        client.prepare_request_buffers(&request).unwrap();
        
        // URL should be converted to UTF-16
        assert!(!client.url_buffer.is_empty());
        
        // Headers should be prepared
        assert!(!client.header_buffer.is_empty());
    }

    #[test]
    fn test_http_client_trait() {
        let mut client = UefiHttpClient::new().unwrap();
        client.initialize_mock().unwrap();
        
        let url = Url::parse("http://example.com/").unwrap();
        let request = Request::get(url);
        
        let response = client.request(&request).unwrap();
        assert!(response.is_success());
    }

    #[test]
    fn test_http_client_trait_with_progress() {
        let mut client = UefiHttpClient::new().unwrap();
        client.initialize_mock().unwrap();
        
        let url = Url::parse("http://example.com/").unwrap();
        let request = Request::get(url);
        
        let response = client.request_with_progress(&request, |transferred, total| {
            assert!(transferred > 0);
            assert!(total.is_some());
        }).unwrap();
        
        assert!(response.is_success());
    }
}
