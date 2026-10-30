mod mcp;

use std::env;
use std::process::ExitCode;

fn main() -> ExitCode {
    let first = env::args_os().nth(1).map(|a| a.to_string_lossy().into_owned()).unwrap_or_default();
    match first.as_str() {
        "mcp" => {
            mcp::serve();
            ExitCode::SUCCESS
        }
        "version" | "--version" => {
            println!("{}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        _ => {
            let version = env!("CARGO_PKG_VERSION");
            eprintln!("lighting {version}: only `mcp` and `version` are built so far, the 2.x client does the rest");
            ExitCode::from(2)
        }
    }
}
