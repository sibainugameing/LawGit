// Installed as `git-law` so that Git discovers it as `git law ...`.
// Note: `git law --help` is intercepted by Git (it looks for a man page);
// use `git law help` instead.
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(lawgit_core::cli::run(args));
}
