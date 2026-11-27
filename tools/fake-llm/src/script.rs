//! The keywords of the last user message (`tools/fake-llm/README.md`, Keywords).
//!
//! Words match whole and in any case; punctuation around a word is ignored. `tool <name>
//! [<json>]` asks for a tool call: the words before it may hold other keywords (`think`, `slow`),
//! and everything after the name is the call's arguments, so `tool` only counts when the name is
//! the last word or a `{` follows it.

/// What the message asks the fake model to do.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Script {
    /// `failed <status>`: answer that HTTP status (400 to 599) instead of a reply.
    pub failed: Option<u16>,
    /// `midstream error`: a little text, then an error event.
    pub midstream: bool,
    /// `slow`: a longer pause between chunks.
    pub slow: bool,
    /// `long`: a long Markdown reply.
    pub long: bool,
    /// `think`: reasoning before the reply.
    pub think: bool,
    /// `refuse`: the reply ends with a refusal.
    pub refuse: bool,
    /// `audio`: a short tone with a transcript (models whose id contains `audio`).
    pub audio: bool,
    pub tool: Option<ToolAsk>,
}

/// `tool <name> <json>`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolAsk {
    /// The name as written; the reply calls the offered tool it names (see `Turn::tool`).
    pub name: String,
    /// The arguments exactly as written (`{}` when none), sent even when they are not JSON.
    pub arguments: String,
}

impl Script {
    pub fn parse(text: &str) -> Self {
        let words = words(text);
        let command = words.iter().enumerate().find_map(|(position, (_, word))| {
            if !word.eq_ignore_ascii_case("tool") {
                return None;
            }
            let (start, name) = words.get(position + 1)?;
            let rest = text[start + name.len()..].trim();
            (rest.is_empty() || rest.starts_with('{')).then(|| {
                let arguments = if rest.is_empty() { "{}" } else { rest };
                (
                    position,
                    ToolAsk {
                        name: trim(name).to_owned(),
                        arguments: arguments.to_owned(),
                    },
                )
            })
        });
        let (scanned, tool) = match command {
            Some((position, ask)) => (&words[..position], Some(ask)),
            None => (&words[..], None),
        };
        let mut script = Self {
            tool,
            ..Self::default()
        };
        for (index, (_, word)) in scanned.iter().enumerate() {
            let next = scanned.get(index + 1).map(|(_, next)| trim(next));
            match trim(word).to_ascii_lowercase().as_str() {
                "failed" => {
                    if script.failed.is_none() {
                        script.failed = next
                            .and_then(|status| status.parse().ok())
                            .filter(|status| (400..=599).contains(status));
                    }
                }
                "midstream" => {
                    script.midstream |= next.is_some_and(|next| next.eq_ignore_ascii_case("error"));
                }
                "slow" => script.slow = true,
                "long" => script.long = true,
                "think" => script.think = true,
                "refuse" => script.refuse = true,
                "audio" => script.audio = true,
                _ => {}
            }
        }
        script
    }
}

/// The whitespace-separated words of `text` with their byte offsets.
fn words(text: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut start = None;
    for (offset, character) in text.char_indices() {
        match (character.is_whitespace(), start) {
            (true, Some(begin)) => {
                out.push((begin, &text[begin..offset]));
                start = None;
            }
            (false, None) => start = Some(offset),
            _ => {}
        }
    }
    if let Some(begin) = start {
        out.push((begin, &text[begin..]));
    }
    out
}

/// A word without the punctuation around it.
fn trim(word: &str) -> &str {
    word.trim_matches(|character: char| {
        !character.is_alphanumeric() && character != '_' && character != '-'
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_asks_for_nothing() {
        assert_eq!(Script::parse("hello there"), Script::default());
        assert_eq!(Script::parse(""), Script::default());
    }

    #[test]
    fn reads_failed_with_an_error_status_only() {
        assert_eq!(Script::parse("failed 403").failed, Some(403));
        assert_eq!(Script::parse("please FAILED 429.").failed, Some(429));
        assert_eq!(
            Script::parse("failed 500 then failed 403").failed,
            Some(500)
        );
        assert_eq!(Script::parse("failed 200").failed, None);
        assert_eq!(Script::parse("failed 600").failed, None);
        assert_eq!(Script::parse("failed").failed, None);
        assert_eq!(Script::parse("failed later").failed, None);
    }

    #[test]
    fn reads_flags_as_whole_words() {
        let script = Script::parse("Think, then be slow and long. Midstream error! refuse audio");
        assert!(script.think && script.slow && script.long && script.refuse && script.audio);
        assert!(script.midstream);
        let partial = Script::parse("thinking slowly about longer audios");
        assert_eq!(partial, Script::default());
        assert!(!Script::parse("midstream").midstream);
        assert!(!Script::parse("midstream errors").midstream);
    }

    #[test]
    fn reads_a_tool_call_with_its_arguments_verbatim() {
        let script = Script::parse(r#"tool echo {"text": "slow long"}"#);
        assert_eq!(
            script.tool,
            Some(ToolAsk {
                name: "echo".to_owned(),
                arguments: r#"{"text": "slow long"}"#.to_owned(),
            })
        );
        assert!(!script.slow && !script.long, "arguments hold no keywords");

        let bare = Script::parse("tool add");
        assert_eq!(bare.tool.expect("a call").arguments, "{}");

        let broken = Script::parse("tool echo {not json");
        assert_eq!(broken.tool.expect("a call").arguments, "{not json");
    }

    #[test]
    fn keeps_keywords_before_a_tool_call() {
        let script = Script::parse(r#"think slow tool add {"a": 1, "b": 2}"#);
        assert!(script.think && script.slow);
        assert_eq!(script.tool.expect("a call").name, "add");
    }

    #[test]
    fn a_tool_word_in_a_sentence_is_not_a_call() {
        assert_eq!(Script::parse("which tool do you have").tool, None);
        assert_eq!(Script::parse("tool").tool, None);
    }
}
