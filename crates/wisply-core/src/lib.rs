mod args;
mod cap;
mod clock;
mod close;
mod fail;
mod log;
mod names;

pub use args::{Call, Flag, parse, wants_help};
pub use cap::{OUT_CHARS, cap, out_file};
pub use clock::{ago, local_now, local_parts, local_to_utc, stamp_secs};
pub use close::close_match;
pub use fail::Fail;
pub use log::{Log, Row, listing, stats};
pub use names::{APP_ONLY, CHAT, Lane, QUIET, SHARED, WEB_ONLY, built_later, lane, unknown};

pub fn ext_version(version: &str) -> &str {
    version.split(['-', '+']).next().unwrap_or(version)
}

#[cfg(test)]
mod version_tests {
    #[test]
    fn the_extension_gets_a_version_chrome_accepts() {
        assert_eq!(super::ext_version("3.0.0-dev.0"), "3.0.0");
        assert_eq!(super::ext_version("3.0.1+build.5"), "3.0.1");
        assert_eq!(super::ext_version("3.1.0"), "3.1.0");
    }
}
