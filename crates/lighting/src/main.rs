mod hook;
mod host;
mod mcp;
mod v2;

use std::env;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = env::args_os().skip(1).map(|a| a.to_string_lossy().into_owned()).collect();
    let first = args.first().map(String::as_str).unwrap_or("");
    match first {
        "mcp" => mcp::serve(),
        "host" => host::serve(args.get(1).map(String::as_str).unwrap_or("")),
        "hook" if args.get(1).is_some_and(|a| a == "post") => hook::post(),
        origin if origin.starts_with("chrome-extension://") => host::serve(origin),
        "version" | "--version" => println!("{}", env!("CARGO_PKG_VERSION")),
        _ => {
            let version = env!("CARGO_PKG_VERSION");
            eprintln!(
                "lighting {version}: only `mcp`, `host`, `hook post` and `version` are built so far, the 2.x client does the rest"
            );
            return ExitCode::from(2);
        }
    }
    ExitCode::SUCCESS
}
