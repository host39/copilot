//! What the fake model answers, before either wire format renders it: the request read into a
//! [`Turn`], and the [`Plan`] for it.

use crate::script::Script;

/// The sample rate of spoken replies (PCM16 mono), as OpenAI streams them.
pub const AUDIO_RATE: u32 = 24_000;
/// The tone of a spoken reply.
const TONE_HZ: f64 = 440.0;
/// How long the tone plays, in milliseconds.
const TONE_MS: u32 = 1_000;
/// The transcript of the tone, streamed in pieces with it.
pub const TONE_TRANSCRIPT: &str = "Here is a short tone.";

/// The request, read the same way for both wire formats.
#[derive(Clone, Debug, Default)]
pub struct Turn {
    pub model: String,
    /// The text of the last user message (its text parts, joined by line breaks).
    pub text: String,
    /// Images, files, and audio clips in the last user message.
    pub attachments: usize,
    /// The tool results the request ends with (their text), when it ends with results.
    pub results: Option<Vec<String>>,
    /// The tool names the request offers.
    pub tools: Vec<String>,
    /// The request turns reasoning on (only counted for models whose id contains `reasoner`).
    pub reasoning: bool,
    /// The request asks for spoken output.
    pub audio: bool,
    /// Characters of request text, for the usage counts.
    pub input_chars: usize,
}

impl Turn {
    /// The offered tool a `tool <name>` command means: the name itself, or a Copilot alias that
    /// ends in `__<name>`. A name nobody offers is called as written, so the client's own
    /// "unknown tool" path can be tried too.
    pub fn tool(&self, name: &str) -> String {
        let alias = format!("__{name}");
        self.tools
            .iter()
            .find(|tool| *tool == name)
            .or_else(|| self.tools.iter().find(|tool| tool.ends_with(&alias)))
            .cloned()
            .unwrap_or_else(|| name.to_owned())
    }
}

/// The newest message of a user message's content parts (`Some(text)` for a text part, `None`
/// for an image, file, or audio clip): its text and how many attachments it has.
///
/// Copilot sends a message's text first and its attachments after it, and merges a user message
/// whose answer it leaves out of the request (refused, or failed before any text) into the next
/// one. So the newest message is the last text part and the parts after it; without one, every
/// part counts.
pub fn newest<'a>(parts: impl IntoIterator<Item = Option<&'a str>>) -> (String, usize) {
    let parts: Vec<Option<&str>> = parts.into_iter().collect();
    match parts.iter().rposition(Option::is_some) {
        Some(position) => (
            parts[position].unwrap_or_default().trim().to_owned(),
            parts.len() - position - 1,
        ),
        None => (String::new(), parts.len()),
    }
}

/// One piece of a reply, in order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Block {
    Reasoning(String),
    Text(String),
    Call {
        id: String,
        name: String,
        arguments: String,
    },
    /// PCM16 mono at [`AUDIO_RATE`], with its transcript.
    Audio {
        pcm: Vec<u8>,
        transcript: String,
    },
}

/// How a reply ends.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Ending {
    Stop,
    ToolUse,
    Refusal,
    /// The stream breaks with an error event after the blocks.
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Reply {
    pub blocks: Vec<Block>,
    pub ending: Ending,
    /// Estimated tokens (four characters each).
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// What the fake model does with a turn.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Plan {
    /// Answer this HTTP status with the format's error body.
    Fail(u16),
    Answer {
        reply: Reply,
        /// Pause longer between chunks.
        slow: bool,
    },
}

/// Decides the answer: tool results first, then `failed`, then the other keywords.
pub fn plan(turn: &Turn, sequence: u64) -> Plan {
    if let Some(results) = &turn.results {
        let text = format!("Tool said: {}", results.join("\n"));
        return Plan::Answer {
            reply: reply(turn, vec![Block::Text(text)], Ending::Stop),
            slow: false,
        };
    }
    let script = Script::parse(&turn.text);
    if let Some(status) = script.failed {
        return Plan::Fail(status);
    }
    let mut blocks = Vec::new();
    if script.think || (turn.reasoning && turn.model.contains("reasoner")) {
        blocks.push(Block::Reasoning(format!(
            "The user wrote {:?}. A fake model has no thoughts, so it pretends to weigh the answer in a few short steps before it replies.",
            turn.text
        )));
    }
    let ending = if let Some(ask) = &script.tool {
        blocks.push(Block::Call {
            id: format!("call_fake_{sequence}"),
            name: turn.tool(&ask.name),
            arguments: ask.arguments.clone(),
        });
        Ending::ToolUse
    } else if script.refuse {
        blocks.push(Block::Text("I would rather not".to_owned()));
        Ending::Refusal
    } else if script.midstream {
        blocks.push(Block::Text("This answer starts well, but".to_owned()));
        Ending::Error
    } else if script.audio && turn.model.contains("audio") {
        blocks.push(if turn.audio {
            Block::Audio {
                pcm: tone(),
                transcript: TONE_TRANSCRIPT.to_owned(),
            }
        } else {
            Block::Text(
                "The request asked for no audio: turn on Speak replies to hear a tone.".to_owned(),
            )
        });
        Ending::Stop
    } else if script.long {
        blocks.push(Block::Text(long_text()));
        Ending::Stop
    } else {
        let mut text = format!("You said: {}", turn.text);
        match turn.attachments {
            0 => {}
            1 => text.push_str(" (with 1 attachment)"),
            count => text.push_str(&format!(" (with {count} attachments)")),
        }
        blocks.push(Block::Text(text));
        Ending::Stop
    };
    Plan::Answer {
        reply: reply(turn, blocks, ending),
        slow: script.slow,
    }
}

fn reply(turn: &Turn, blocks: Vec<Block>, ending: Ending) -> Reply {
    let output: usize = blocks
        .iter()
        .map(|block| match block {
            Block::Reasoning(text) | Block::Text(text) => text.len(),
            Block::Call { arguments, .. } => arguments.len(),
            Block::Audio { transcript, .. } => transcript.len(),
        })
        .sum();
    Reply {
        blocks,
        ending,
        input_tokens: tokens(turn.input_chars),
        output_tokens: tokens(output),
    }
}

/// Four characters per token, at least one.
fn tokens(chars: usize) -> u32 {
    u32::try_from(chars.div_ceil(4).max(1)).unwrap_or(u32::MAX)
}

/// A long Markdown reply: a heading, paragraphs, a list, and a code block.
pub fn long_text() -> String {
    let mut text = String::from("# A Long Reply\n\n");
    for paragraph in 1..=24 {
        text.push_str(&format!(
            "Paragraph {paragraph}. The fake model keeps talking so the chat view has something to scroll through. \
             Every sentence here is filler, written to test how a long streamed answer renders, wraps, and stays readable \
             while more of it arrives in small pieces.\n\n"
        ));
        if paragraph == 12 {
            text.push_str("Some points in the middle:\n\n- the first point\n- a second point\n- a third point\n\n");
        }
    }
    text.push_str("```rust\nfn main() {\n    let reply = \"long\";\n    assert_eq!(reply.len(), 4);\n}\n```\n\nThe end.");
    text
}

/// A short sine tone: PCM16 little-endian mono at [`AUDIO_RATE`], faded in and out.
pub fn tone() -> Vec<u8> {
    let samples = AUDIO_RATE * TONE_MS / 1_000;
    let fade = f64::from(AUDIO_RATE / 100);
    let mut pcm = Vec::with_capacity(samples as usize * 2);
    for index in 0..samples {
        let time = f64::from(index) / f64::from(AUDIO_RATE);
        let edge = f64::from(index.min(samples - 1 - index));
        let envelope = (edge / fade).min(1.0);
        let value = (time * TONE_HZ * std::f64::consts::TAU).sin() * envelope * 0.3;
        // In range: |value| <= 0.3, so the product fits an i16.
        #[allow(clippy::cast_possible_truncation)]
        let sample = (value * f64::from(i16::MAX)) as i16;
        pcm.extend_from_slice(&sample.to_le_bytes());
    }
    pcm
}

/// `pcm` (PCM16 mono at [`AUDIO_RATE`]) in a WAV container.
pub fn wav(pcm: &[u8]) -> Vec<u8> {
    let data = u32::try_from(pcm.len()).unwrap_or(u32::MAX);
    let mut out = Vec::with_capacity(pcm.len() + 44);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16_u32.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&AUDIO_RATE.to_le_bytes());
    out.extend_from_slice(&(AUDIO_RATE * 2).to_le_bytes());
    out.extend_from_slice(&2_u16.to_le_bytes());
    out.extend_from_slice(&16_u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    out.extend_from_slice(pcm);
    out
}

/// The signature of a thinking block: a hash of its text, so a replayed block that was changed
/// or lost its signature can be told apart from one sent back verbatim.
pub fn signature(thinking: &str) -> String {
    // FNV-1a, 64 bits.
    let hash = thinking
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
        });
    format!("fake-signature-{hash:016x}")
}

/// `text` in streaming pieces: one word each for short texts, five for long ones, whitespace
/// kept with the word before it.
pub fn chunks(text: &str) -> Vec<String> {
    let per = if text.split_whitespace().count() < 50 {
        1
    } else {
        5
    };
    let mut out = Vec::new();
    let mut current = String::new();
    let mut words = 0;
    let mut in_word = false;
    for character in text.chars() {
        let space = character.is_whitespace();
        if !space && !in_word {
            if words == per {
                out.push(std::mem::take(&mut current));
                words = 0;
            }
            words += 1;
        }
        in_word = !space;
        current.push(character);
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(text: &str) -> Turn {
        Turn {
            model: "fake-chat".to_owned(),
            text: text.to_owned(),
            input_chars: text.len(),
            ..Turn::default()
        }
    }

    fn answer(plan: Plan) -> (Reply, bool) {
        match plan {
            Plan::Answer { reply, slow } => (reply, slow),
            Plan::Fail(status) => panic!("failed with {status}"),
        }
    }

    #[test]
    fn echoes_plain_text_with_usage() {
        let (reply, slow) = answer(plan(&turn("hello"), 1));
        assert_eq!(
            reply.blocks,
            vec![Block::Text("You said: hello".to_owned())]
        );
        assert_eq!(reply.ending, Ending::Stop);
        assert_eq!((reply.input_tokens, reply.output_tokens), (2, 4));
        assert!(!slow);
    }

    #[test]
    fn counts_attachments() {
        let mut one = turn("look");
        one.attachments = 1;
        let (reply, _) = answer(plan(&one, 1));
        assert_eq!(
            reply.blocks,
            vec![Block::Text("You said: look (with 1 attachment)".to_owned())]
        );
        one.attachments = 3;
        let (reply, _) = answer(plan(&one, 1));
        assert_eq!(
            reply.blocks,
            vec![Block::Text(
                "You said: look (with 3 attachments)".to_owned()
            )]
        );
    }

    #[test]
    fn failed_wins_over_other_keywords() {
        assert_eq!(plan(&turn("think slow failed 429"), 1), Plan::Fail(429));
    }

    #[test]
    fn tool_results_answer_with_their_text() {
        let mut ended = turn("failed 500");
        ended.results = Some(vec!["one".to_owned(), "two".to_owned()]);
        let (reply, _) = answer(plan(&ended, 1));
        assert_eq!(
            reply.blocks,
            vec![Block::Text("Tool said: one\ntwo".to_owned())]
        );
    }

    #[test]
    fn calls_the_offered_tool_by_name_or_alias() {
        let mut ask = turn(r#"think tool echo {"text":"hi"}"#);
        ask.tools = vec!["fake__add".to_owned(), "fake__echo".to_owned()];
        let (reply, _) = answer(plan(&ask, 7));
        assert!(matches!(reply.blocks[0], Block::Reasoning(_)));
        assert_eq!(
            reply.blocks[1],
            Block::Call {
                id: "call_fake_7".to_owned(),
                name: "fake__echo".to_owned(),
                arguments: r#"{"text":"hi"}"#.to_owned(),
            }
        );
        assert_eq!(reply.ending, Ending::ToolUse);
        assert_eq!(ask.tool("missing"), "missing");
        ask.tools.push("echo".to_owned());
        assert_eq!(ask.tool("echo"), "echo", "an exact name wins over an alias");
    }

    #[test]
    fn refuses_breaks_and_runs_long() {
        assert_eq!(answer(plan(&turn("refuse"), 1)).0.ending, Ending::Refusal);
        assert_eq!(
            answer(plan(&turn("midstream error"), 1)).0.ending,
            Ending::Error
        );
        let (reply, slow) = answer(plan(&turn("slow long"), 1));
        assert!(slow);
        let Block::Text(text) = &reply.blocks[0] else {
            panic!("a text block");
        };
        assert!(text.len() > 5_000 && text.starts_with("# A Long Reply"));
    }

    #[test]
    fn reasoner_models_reason_when_the_request_asks() {
        let mut asked = turn("hello");
        asked.reasoning = true;
        assert_eq!(
            answer(plan(&asked, 1)).0.blocks.len(),
            1,
            "fake-chat never reasons unasked"
        );
        asked.model = "fake-reasoner".to_owned();
        assert!(matches!(
            answer(plan(&asked, 1)).0.blocks[0],
            Block::Reasoning(_)
        ));
    }

    #[test]
    fn speaks_only_on_audio_models_that_asked_for_audio() {
        let mut spoken = turn("audio please");
        assert_eq!(
            answer(plan(&spoken, 1)).0.blocks,
            vec![Block::Text("You said: audio please".to_owned())]
        );
        spoken.model = "fake-audio".to_owned();
        assert!(
            matches!(&answer(plan(&spoken, 1)).0.blocks[0], Block::Text(text) if text.contains("Speak replies"))
        );
        spoken.audio = true;
        let (reply, _) = answer(plan(&spoken, 1));
        let Block::Audio { pcm, transcript } = &reply.blocks[0] else {
            panic!("an audio block");
        };
        assert_eq!(pcm.len(), 48_000, "one second of 24 kHz PCM16");
        assert_eq!(transcript, TONE_TRANSCRIPT);
    }

    #[test]
    fn the_tone_fades_in_and_stays_in_range() {
        let pcm = tone();
        let samples: Vec<i16> = pcm
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| i16::from_le_bytes(*pair))
            .collect();
        assert_eq!(samples[0], 0);
        let peak = samples
            .iter()
            .map(|sample| sample.unsigned_abs())
            .max()
            .expect("samples");
        assert!(
            peak > 9_000 && peak < 10_000,
            "about 0.3 of full scale, got {peak}"
        );
    }

    #[test]
    fn wraps_pcm_in_a_wav_header() {
        let file = wav(&[1, 0, 2, 0]);
        assert_eq!(&file[..4], b"RIFF");
        assert_eq!(&file[8..16], b"WAVEfmt ");
        assert_eq!(
            u32::from_le_bytes([file[24], file[25], file[26], file[27]]),
            AUDIO_RATE
        );
        assert_eq!(
            u32::from_le_bytes([file[40], file[41], file[42], file[43]]),
            4
        );
        assert_eq!(file.len(), 48);
    }

    #[test]
    fn the_newest_message_is_the_last_text_and_what_follows() {
        assert_eq!(newest([Some(" hello ")]), ("hello".to_owned(), 0));
        assert_eq!(
            newest([Some("refuse"), None, Some("hello"), None, None]),
            ("hello".to_owned(), 2),
            "a merged earlier message and its attachment are not counted"
        );
        assert_eq!(newest([None, None]), (String::new(), 2));
        assert_eq!(newest([]), (String::new(), 0));
    }

    #[test]
    fn signatures_follow_the_text() {
        assert_eq!(signature("a"), signature("a"));
        assert_ne!(signature("a"), signature("b"));
        assert!(signature("").starts_with("fake-signature-"));
    }

    #[test]
    fn chunks_keep_every_character() {
        assert_eq!(chunks("You said: hello"), vec!["You ", "said: ", "hello"]);
        let long = long_text();
        let pieces = chunks(&long);
        assert!(pieces.len() > 50);
        assert_eq!(pieces.concat(), long);
        assert_eq!(chunks(""), Vec::<String>::new());
    }
}
