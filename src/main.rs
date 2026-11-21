//! Command-line entry point: hands the arguments to `cli::main`, which owns dispatch, help, and
//! error reporting, and exits with the code it returns (0 ok, 1 failure, 2 usage error).

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(aether_motion::cli::main(&args));
}
