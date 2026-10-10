#[derive(Default)]
enum Escape {
    #[default]
    None,
    Slash,
    Unicode { value: u16, digits: u8, high: Option<u16> },
    LowSlash(u16),
    LowU(u16),
}

#[derive(Default)]
pub struct JsonString {
    pub value: String,
    escape: Escape,
}

impl JsonString {
    pub fn push(&mut self, character: char) -> Result<bool, ()> {
        let escape = std::mem::take(&mut self.escape);
        match escape {
            Escape::None => match character {
                '"' => return Ok(true),
                '\\' => self.escape = Escape::Slash,
                c if c < '\u{20}' => return Err(()),
                c => self.value.push(c),
            },
            Escape::Slash => match character {
                '"' | '\\' | '/' => self.value.push(character),
                'n' => self.value.push('\n'),
                'r' => self.value.push('\r'),
                't' => self.value.push('\t'),
                'b' => self.value.push('\u{8}'),
                'f' => self.value.push('\u{c}'),
                'u' => self.escape = Escape::Unicode { value: 0, digits: 0, high: None },
                _ => return Err(()),
            },
            Escape::Unicode { value, digits, high } => {
                let hex = character.to_digit(16).ok_or(())? as u16;
                let value = (value << 4) | hex;
                let digits = digits + 1;
                if digits < 4 {
                    self.escape = Escape::Unicode { value, digits, high };
                } else if let Some(high) = high {
                    if !(0xDC00..=0xDFFF).contains(&value) { return Err(()); }
                    let scalar = 0x10000 + (((high as u32 - 0xD800) << 10) | (value as u32 - 0xDC00));
                    self.value.push(char::from_u32(scalar).ok_or(())?);
                } else if (0xD800..=0xDBFF).contains(&value) {
                    self.escape = Escape::LowSlash(value);
                } else {
                    self.value.push(char::from_u32(value as u32).ok_or(())?);
                }
            }
            Escape::LowSlash(high) => {
                if character != '\\' { return Err(()); }
                self.escape = Escape::LowU(high);
            }
            Escape::LowU(high) => {
                if character != 'u' { return Err(()); }
                self.escape = Escape::Unicode { value: 0, digits: 0, high: Some(high) };
            }
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(raw: &str) -> Result<String, ()> {
        let mut decoder = JsonString::default();
        for character in raw.chars() {
            if decoder.push(character)? { return Ok(decoder.value); }
        }
        Err(())
    }

    #[test]
    fn decodes_unicode_surrogates_and_escaped_controls_incrementally() {
        assert_eq!(decode(r#"Line\n\"quote\" \\ \u65e5\u672c \ud83d\ude42""#).unwrap(),
            "Line\n\"quote\" \\ \u{65e5}\u{672c} \u{1f642}");
        assert_eq!(decode("\u{65e5}\u{1f642}\"").unwrap(), "\u{65e5}\u{1f642}");
    }

    #[test]
    fn malformed_escapes_and_unpaired_surrogates_are_rejected() {
        for raw in [r#"\uD83Dx""#, r#"\uDE42""#, r#"\uD83D\u0041""#, r#"\x""#, "raw\nnewline\""] {
            assert!(decode(raw).is_err(), "{raw}");
        }
    }

    #[test]
    fn incomplete_escape_does_not_emit_partial_unicode() {
        let mut decoder = JsonString::default();
        for character in r"\ud83d\ude4".chars() {
            assert!(!decoder.push(character).unwrap());
            assert!(decoder.value.is_empty());
        }
        decoder.push('2').unwrap();
        assert_eq!(decoder.value, "\u{1f642}");
        assert!(decoder.push('"').unwrap());
    }
}
