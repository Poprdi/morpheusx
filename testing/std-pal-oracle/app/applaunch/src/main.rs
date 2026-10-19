//! /bin/init for the ecosystem-proof images. The kernel spawns init with no
//! argv, so this plays the shell: it runs each line of /corpus/cmds.txt as a
//! command (argv split on whitespace) with stdout/stderr inherited, so the real
//! app's output lands on serial unaltered.
use std::process::Command;

fn main() {
    let cmds = std::fs::read_to_string("/corpus/cmds.txt").expect("read /corpus/cmds.txt");
    let (mut ok, mut n) = (0, 0);
    for line in cmds.lines().filter(|l| !l.trim().is_empty() && !l.starts_with('#')) {
        let argv: Vec<&str> = line.split_whitespace().collect();
        n += 1;
        println!("\n$ {}", argv.join(" "));
        match Command::new(argv[0]).args(&argv[1..]).status() {
            Ok(st) => {
                println!("[exit {:?}]", st.code());
                if st.code() == Some(0) {
                    ok += 1;
                }
            }
            Err(e) => println!("[spawn failed: {e}]"),
        }
    }
    println!("\nMORPH_APP_DONE ok={ok}/{n}");
}
