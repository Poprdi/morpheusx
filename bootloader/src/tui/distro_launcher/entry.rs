use alloc::string::String;

#[derive(Clone)]
pub struct BootEntry {
    pub name: String,
    pub kernel_path: String,
    pub initrd_path: Option<String>,
    pub cmdline: String,
    pub root_device: Option<String>,
}

impl BootEntry {
    pub fn new(
        name: String,
        kernel_path: String,
        initrd_path: Option<String>,
        cmdline: String,
    ) -> Self {
        Self {
            name,
            kernel_path,
            initrd_path,
            cmdline,
            root_device: None,
        }
    }

    pub fn with_root(mut self, root: String) -> Self {
        self.root_device = Some(root);
        self
    }
}
