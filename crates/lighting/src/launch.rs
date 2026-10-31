use std::fs;
use std::process::{Command, ExitCode};

use serde_json::Value;

pub fn main(args: &[String]) -> ExitCode {
    let Some(file) = args.get(1) else { return ExitCode::FAILURE };
    let raw = fs::read_to_string(file).unwrap_or_default();
    let _ = fs::remove_file(file);
    let spec: Value = serde_json::from_str(&raw).unwrap_or_default();
    let Some(exe) = spec["exe"].as_str() else {
        eprintln!("lighting: no start file at {file}");
        return ExitCode::FAILURE;
    };
    let argv: Vec<&str> = spec["args"].as_array().map_or(Vec::new(), |a| a.iter().filter_map(Value::as_str).collect());
    match Command::new(exe).args(argv).status() {
        Ok(s) => ExitCode::from(u8::try_from(s.code().unwrap_or(1)).unwrap_or(1)),
        Err(e) => {
            eprintln!("lighting: could not start {exe}: {e}");
            ExitCode::FAILURE
        }
    }
}
