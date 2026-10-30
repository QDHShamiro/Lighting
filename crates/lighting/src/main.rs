mod client;
mod daemon;
mod hook;
mod host;
mod mcp;
mod more;
mod pipe;
mod spawn;
mod v2;
#[cfg(windows)]
mod voice;

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
        "daemon" if daemon::may_serve() => return daemon::serve(),
        #[cfg(windows)]
        "voice-session" | "voice-watch" => return voice::main(&args),
        _ => return client::run(&args),
    }
    ExitCode::SUCCESS
}
