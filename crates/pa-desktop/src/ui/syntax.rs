//! Lightweight, lossless token colouring for transcript code. Offsets and run
//! lengths always refer to the original UTF-8 source, including newlines.

use gpui::{TextRun, font};

use crate::theme::{Palette, font_mono};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Language {
    Rust,
    Python,
    JavaScript,
    Shell,
    Json,
    Config,
    C,
    Go,
    Html,
    Css,
    Sql,
    Markdown,
    Diff,
    Plain,
}

impl Language {
    fn named(name: &str) -> Self {
        match name.to_ascii_lowercase().as_str() {
            "rs" | "rust" => Self::Rust,
            "py" | "python" | "python3" | "python_repl" => Self::Python,
            "js" | "jsx" | "mjs" | "javascript" | "ts" | "tsx" | "typescript" => Self::JavaScript,
            "shell" | "sh" | "bash" | "zsh" | "console" => Self::Shell,
            "json" | "jsonc" => Self::Json,
            "toml" | "yaml" | "yml" => Self::Config,
            "c" | "h" | "c++" | "cpp" | "cxx" | "hpp" => Self::C,
            "go" | "golang" => Self::Go,
            "html" | "xml" | "svg" => Self::Html,
            "css" | "scss" => Self::Css,
            "sql" | "postgresql" | "sqlite" => Self::Sql,
            "md" | "markdown" => Self::Markdown,
            "diff" | "patch" => Self::Diff,
            _ => Self::Plain,
        }
    }

    fn keyword(self, word: &str) -> bool {
        let words = match self {
            Self::Rust => {
                "as async await break const continue crate dyn else enum extern fn for if impl in let loop match mod move mut pub ref return self Self static struct super trait type unsafe use where while yield"
            }
            Self::Python => {
                "and as assert async await break class continue def del elif else except finally for from global if import in is lambda nonlocal not or pass raise return try while with yield match case"
            }
            Self::JavaScript => {
                "abstract as async await break case catch class const continue declare default delete do else enum export extends finally for from function get if implements import in instanceof interface let new of private public readonly return set static switch this throw try type typeof var void while yield"
            }
            Self::Shell => {
                "break case continue do done echo elif else esac exit export fi for function if in local printf read return set shift then until while"
            }
            Self::C => {
                "alignas auto bool break case catch char class const constexpr continue default define delete do double else enum explicit extern float for if include int long namespace new private protected public return short signed sizeof static struct switch template this throw try typedef typename union unsigned using virtual void while"
            }
            Self::Go => {
                "break case chan const continue default defer else fallthrough for func go goto if import interface map package range return select struct switch type var"
            }
            Self::Sql => {
                "all alter and as asc begin by case commit count create delete desc distinct drop else end exists from full group having in inner insert into is join left like limit not on or order outer primary right rollback select set table then union unique update values when where with"
            }
            _ => return false,
        };
        if self == Self::Sql {
            words
                .split_ascii_whitespace()
                .any(|key| key.eq_ignore_ascii_case(word))
        } else {
            words.split_ascii_whitespace().any(|key| key == word)
        }
    }

    fn slash_comments(self) -> bool {
        matches!(
            self,
            Self::Rust | Self::JavaScript | Self::C | Self::Go | Self::Json
        )
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Token {
    Plain,
    Keyword,
    String,
    Number,
    Name,
    Comment,
    Punctuation,
    Added,
    Removed,
    Context,
}

fn run(token: Token, len: usize, p: Palette) -> TextRun {
    let color = match token {
        Token::Plain => p.text_primary,
        Token::Keyword => p.accent,
        Token::String | Token::Added => p.success,
        Token::Number => p.warning,
        Token::Name => p.info,
        Token::Comment => p.text_muted,
        Token::Punctuation | Token::Context => p.text_secondary,
        Token::Removed => p.danger,
    };
    TextRun {
        len,
        font: font(font_mono()),
        color,
        background_color: match token {
            Token::Added => Some(p.success_soft),
            Token::Removed => Some(p.danger_soft),
            _ => None,
        },
        underline: None,
        strikethrough: None,
    }
}

/// A complete byte partition ready to pass to GPUI's styled text.
#[must_use]
pub(super) fn highlight(lang: &str, source: &str, palette: Palette) -> Vec<TextRun> {
    let language = Language::named(lang);
    if language == Language::Plain {
        return (!source.is_empty())
            .then(|| run(Token::Plain, source.len(), palette))
            .into_iter()
            .collect();
    }
    if language == Language::Diff {
        return source
            .split_inclusive('\n')
            .map(|line| {
                let token = if line.starts_with("@@")
                    || line.starts_with("+++ ")
                    || line.starts_with("--- ")
                    || line.starts_with("diff --git ")
                {
                    Token::Name
                } else if line.starts_with('+') {
                    Token::Added
                } else if line.starts_with('-') {
                    Token::Removed
                } else {
                    Token::Context
                };
                run(token, line.len(), palette)
            })
            .collect();
    }
    let mut lexer = Lexer {
        source,
        pos: 0,
        language,
        in_tag: false,
    };
    let mut runs: Vec<TextRun> = Vec::new();
    let mut previous = None;
    while lexer.pos < source.len() {
        let (token, len) = lexer.next();
        if let Some(last) = runs.last_mut()
            && previous == Some(token)
        {
            last.len += len;
        } else {
            runs.push(run(token, len, palette));
        }
        previous = Some(token);
        lexer.pos += len;
    }
    runs
}

struct Lexer<'a> {
    source: &'a str,
    pos: usize,
    language: Language,
    in_tag: bool,
}

impl Lexer<'_> {
    fn next(&mut self) -> (Token, usize) {
        let rest = &self.source[self.pos..];
        let lang = self.language;
        let first = rest.as_bytes()[0];
        if first.is_ascii_whitespace() {
            return (
                Token::Plain,
                rest.bytes().take_while(u8::is_ascii_whitespace).count(),
            );
        }
        if let Some(len) = self.comment_len(rest) {
            return (Token::Comment, len);
        }
        if matches!(lang, Language::Html | Language::JavaScript) {
            if rest.starts_with('<')
                && rest
                    .as_bytes()
                    .get(1)
                    .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'/')
            {
                self.in_tag = true;
            } else if first == b'>' {
                self.in_tag = false;
            }
        }
        if lang == Language::Rust {
            if rest.starts_with("#[") || rest.starts_with("#![") {
                let prefix = rest.find('[').expect("attribute opener") + 1;
                return (Token::Name, prefix + identifier_len(&rest[prefix..]));
            }
            if let Some(len) = rust_raw_string(rest) {
                return (Token::String, len);
            }
            if first == b'\'' {
                let ident = identifier_len(&rest[1..]);
                if ident > 0 && !rest[1 + ident..].starts_with('\'') {
                    return (Token::Name, 1 + ident);
                }
            }
        }
        if matches!(first, b'\'' | b'"')
            || first == b'`'
                && matches!(
                    lang,
                    Language::JavaScript | Language::Go | Language::Shell | Language::Markdown
                )
        {
            let len = quoted_len(rest, lang);
            return (
                if rest[len..].trim_start_matches([' ', '\t']).starts_with(':')
                    && matches!(lang, Language::Json | Language::Config)
                {
                    Token::Name
                } else {
                    Token::String
                },
                len,
            );
        }
        if lang == Language::Markdown
            && first == b'#'
            && (self.pos == 0 || self.source.as_bytes()[self.pos - 1] == b'\n')
        {
            return (Token::Keyword, rest.find('\n').unwrap_or(rest.len()));
        }
        if first.is_ascii_digit() {
            return (Token::Number, number_len(rest));
        }
        if matches!(first, b'$' | b'@' | b'#')
            && matches!(
                lang,
                Language::Css | Language::Shell | Language::C | Language::Python
            )
        {
            return (Token::Name, 1 + identifier_len(&rest[1..]));
        }
        let len = identifier_len(rest);
        if len > 0 {
            let word = &rest[..len];
            let next = rest[len..].trim_start_matches([' ', '\t']);
            let token = if lang.keyword(word) {
                Token::Keyword
            } else if matches!(
                word,
                "None" | "true" | "false" | "null" | "True" | "False" | "nil" | "undefined"
            ) {
                Token::Number
            } else if self.in_tag
                || next.starts_with('(')
                || (matches!(lang, Language::Config | Language::Css)
                    && next.starts_with([':', '=']))
                || matches!(lang, Language::Rust | Language::C | Language::Go)
                    && word.starts_with(char::is_uppercase)
            {
                Token::Name
            } else {
                Token::Plain
            };
            return (token, len);
        }
        let len = rest.chars().next().expect("nonempty token").len_utf8();
        (
            if first.is_ascii_punctuation() {
                Token::Punctuation
            } else {
                Token::Plain
            },
            len,
        )
    }

    fn comment_len(&self, rest: &str) -> Option<usize> {
        let lang = self.language;
        if lang == Language::Html && rest.starts_with("<!--") {
            return Some(rest[4..].find("-->").map_or(rest.len(), |i| i + 7));
        }
        if (lang.slash_comments() || matches!(lang, Language::Css | Language::Sql))
            && rest.starts_with("/*")
        {
            return Some(rest[2..].find("*/").map_or(rest.len(), |i| i + 4));
        }
        let hash_comment = matches!(lang, Language::Python | Language::Config)
            || lang == Language::Shell
                && (self.pos == 0 || self.source.as_bytes()[self.pos - 1].is_ascii_whitespace());
        if hash_comment && rest.starts_with('#')
            || lang.slash_comments() && rest.starts_with("//")
            || lang == Language::Sql && rest.starts_with("--")
        {
            Some(rest.find('\n').unwrap_or(rest.len()))
        } else {
            None
        }
    }
}

fn identifier_len(s: &str) -> usize {
    s.char_indices()
        .find(|(_, c)| !(c.is_alphanumeric() || *c == '_'))
        .map_or(s.len(), |(i, _)| i)
}

fn number_len(s: &str) -> usize {
    let bytes = s.as_bytes();
    let mut i = 1;
    while let Some(&b) = bytes.get(i) {
        if b.is_ascii_alphanumeric()
            || b == b'_'
            || b == b'.' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit)
            || matches!(b, b'+' | b'-') && matches!(bytes[i - 1], b'e' | b'E')
        {
            i += 1;
        } else {
            break;
        }
    }
    i
}

fn rust_raw_string(rest: &str) -> Option<usize> {
    let body = rest.strip_prefix('r')?;
    let hashes = body.bytes().take_while(|b| *b == b'#').count();
    let body = body[hashes..].strip_prefix('"')?;
    let close = format!("\"{}", "#".repeat(hashes));
    Some(
        body.find(&close)
            .map_or(rest.len(), |i| 2 + hashes + i + close.len()),
    )
}

fn quoted_len(s: &str, language: Language) -> usize {
    let bytes = s.as_bytes();
    let quote = bytes[0];
    let delimiter = if language == Language::Python && bytes.starts_with(&[quote; 3]) {
        3
    } else {
        1
    };
    let mut i = delimiter;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i = (i + 2).min(bytes.len());
        } else if bytes[i..].starts_with(&bytes[..delimiter]) {
            return i + delimiter;
        } else if bytes[i] == b'\n'
            && delimiter == 1
            && !matches!(
                language,
                Language::JavaScript
                    | Language::Rust
                    | Language::Go
                    | Language::Shell
                    | Language::Sql
            )
        {
            break;
        } else {
            i += 1;
        }
    }
    i
}

/// Strip terminal CSI/OSC controls, including hyperlinks terminated by BEL or ST.
/// Preserve visible text and LF so console line numbering and copied output agree.
#[must_use]
pub(super) fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\r' => {}
            '\x1b' => match chars.peek() {
                Some('[') => {
                    chars.next();
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    chars.next();
                    while let Some(c) = chars.next() {
                        if c == '\x07' || c == '\x1b' && chars.next_if_eq(&'\\').is_some() {
                            break;
                        }
                    }
                }
                _ => {}
            },
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::ThemeMode;

    #[test]
    fn languages_color_tokens_and_preserve_byte_boundaries_in_both_themes() {
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            let p = Palette::for_mode(mode);
            for (lang, source, probe, ink) in [
                ("rust", "#[derive(Clone)]\nfn 鳥() {}", "#[derive", p.info),
                ("python", "n = 12 # café", "# café", p.text_muted),
                ("tsx", "const view = <Tag name=\"猫\"/>", "const", p.accent),
                ("bash", "echo \"$USER\"", "\"$USER\"", p.success),
                ("json", "{\"name\": false}", "\"name\"", p.info),
                ("toml", "enabled = true", "enabled", p.info),
                ("yaml", "age: 18", "18", p.warning),
                ("go", "func main() {}", "func", p.accent),
                ("c++", "static int x;", "static", p.accent),
                ("html", "<b>text</b>", "b", p.info),
                ("css", "body { color: #fff; }", "color", p.info),
                ("sql", "SELECT * from x", "SELECT", p.accent),
                ("md", "# Hello\nwords", "# Hello", p.accent),
                ("diff", "@@ -1 +1 @@\n-old\n+new", "@@", p.info),
            ] {
                let runs = highlight(lang, source, p);
                let mut start = 0;
                let index = source.find(probe).unwrap();
                let mut found = None;
                for run in runs {
                    assert_eq!(run.font.family, font_mono());
                    assert!(
                        source.is_char_boundary(start) && source.is_char_boundary(start + run.len)
                    );
                    if (start..start + run.len).contains(&index) {
                        found = Some(run.color);
                    }
                    start += run.len;
                }
                assert_eq!(start, source.len(), "{lang}");
                assert_eq!(found, Some(ink), "{lang}: {probe}");
            }
        }
    }

    #[test]
    fn terminal_controls_strip_without_losing_links_text_or_empty_lines() {
        assert_eq!(
            strip_ansi(
                "\x1b[1;32mgreen\x1b[0m\r\n\n\x1b]8;;https://a\x07猫\x1b]8;;\x1b\\ end\x1b[2K"
            ),
            "green\n\n猫 end"
        );
    }
}
