use libmorpheus::env;
use libmorpheus::io;

pub fn render(last_status: i32) {
    let cwd = env::current_dir().unwrap_or_else(|_| alloc::string::String::from("/"));

    io::print("morpheus:");
    io::print(&cwd);

    if last_status != 0 {
        libmorpheus::print!(" [{}]", last_status);
    }

    io::print("> ");
}
