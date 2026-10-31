#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fail {
    pub msg: String,
    pub hint: Option<String>,
    pub said: bool,
    pub counted: bool,
}

impl Fail {
    pub fn new(msg: impl Into<String>) -> Fail {
        Fail { msg: msg.into(), hint: None, said: false, counted: true }
    }

    pub fn hint(msg: impl Into<String>, hint: impl Into<String>) -> Fail {
        Fail { msg: msg.into(), hint: Some(hint.into()), said: false, counted: true }
    }

    pub fn said(text: impl Into<String>) -> Fail {
        Fail { msg: text.into(), hint: None, said: true, counted: true }
    }

    pub fn uncounted(text: impl Into<String>) -> Fail {
        Fail { msg: text.into(), hint: None, said: true, counted: false }
    }

    pub fn text(&self) -> String {
        match self.hint.as_deref() {
            _ if self.said => self.msg.clone(),
            Some(hint) if !hint.is_empty() => format!("err: {} -> try: {hint}", self.msg),
            _ => format!("err: {}", self.msg),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texts_read_like_the_2x_errors() {
        assert_eq!(Fail::new("no command").text(), "err: no command");
        assert_eq!(Fail::hint("no command", "wisply help").text(), "err: no command -> try: wisply help");
        assert_eq!(Fail::hint("x", "").text(), "err: x");
        assert_eq!(Fail::said("! STOP").text(), "! STOP");
        let timeout = Fail::uncounted("err: browser did not answer 'snap' within 30s");
        assert!(!timeout.counted && Fail::new("x").counted);
        assert_eq!(timeout.text(), "err: browser did not answer 'snap' within 30s");
    }
}
