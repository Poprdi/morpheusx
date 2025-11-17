mod types;
mod scan;
mod create_modify;
mod find;
mod utils;

pub use types::{FreeRegion, GptError};
pub use scan::scan_partitions;
pub use create_modify::{create_gpt, create_partition, shrink_partition, delete_partition};
pub use find::find_free_space;
pub use utils::{align_lba, mb_to_lba, calculate_total_free_space};
