fn main() {
    std::process::exit(workroam_cli::main_with_args(std::env::args_os().collect()));
}
