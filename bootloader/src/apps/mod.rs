pub mod storage_manager;
pub mod task_manager;

use morpheus_ui::app::AppRegistry;

pub fn register_all(registry: &mut AppRegistry) {
    storage_manager::register(registry);
    task_manager::register(registry);
}
