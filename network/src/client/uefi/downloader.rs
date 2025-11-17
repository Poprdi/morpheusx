//! High-level download manager

use crate::client::HttpClient;
use crate::error::Result;
use crate::types::ProgressCallback;
use alloc::vec::Vec;

pub struct Downloader<'a> {
    client: &'a mut dyn HttpClient,
}

impl<'a> Downloader<'a> {
    pub fn new(client: &'a mut dyn HttpClient) -> Self {
        Self { client }
    }

    pub fn download(&mut self, _url: &str) -> Result<Vec<u8>> {
        // TODO: High-level download
        // 1. Parse URL
        // 2. Create Request
        // 3. Execute via client
        // 4. Return body
        todo!("Implement download")
    }

    pub fn download_with_progress(
        &mut self,
        _url: &str,
        _progress: ProgressCallback,
    ) -> Result<Vec<u8>> {
        // TODO: Download with progress
        todo!("Implement download_with_progress")
    }

    pub fn get_file_size(&mut self, _url: &str) -> Result<Option<usize>> {
        // TODO: HEAD request to get Content-Length
        todo!("Implement get_file_size")
    }
}
