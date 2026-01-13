// Clean MORPHEUSX ASCII art logo (raw, without spacing)
pub const LOGO_LINES_RAW: &[&str] = &[
    "███╗   ███╗ ██████╗ ██████╗ ██████╗ ██╗  ██╗███████╗██╗   ██╗███████╗██╗  ██╗",
    "████╗ ████║██╔═══██╗██╔══██╗██╔══██╗██║  ██║██╔════╝██║   ██║██╔════╝╚██╗██╔╝",
    "██╔████╔██║██║   ██║██████╔╝██████╔╝███████║█████╗  ██║   ██║███████╗ ╚███╔╝ ",
    "██║╚██╔╝██║██║   ██║██╔══██╗██╔═══╝ ██╔══██║██╔══╝  ██║   ██║╚════██║ ██╔██╗ ",
    "██║ ╚═╝ ██║╚██████╔╝██║  ██║██║     ██║  ██║███████╗╚██████╔╝███████║██╔╝ ██╗",
    "╚═╝     ╚═╝ ╚═════╝ ╚═╝  ╚═╝╚═╝     ╚═╝  ╚═╝╚══════╝ ╚═════╝ ╚══════╝╚═╝  ╚═╝",
];

pub const LOGO_WIDTH: usize = 79;
pub const LOGO_HEIGHT: usize = 6;

pub const TAGLINE: &str = "Bootloader v1.0.1";
pub const TAGLINE_WIDTH: usize = 32;

// Status messages (raw, without spacing)
pub const STATUS_MSGS_RAW: &[&str] = &[
    ">> Initializing quantum flux capacitors............ [OK]",
    ">> Loading matrix......................... [OK]",
    ">> Foo/Bar loading.................... [OK]",
    "",
    "> Press any key to enter the Matrix...",
];

pub const STATUS_WIDTH: usize = 54;
