//! OpenAI CLIP's byte-level BPE tokenizer, for SAM 3's text encoder (ADR 0025), following the
//! reference export's copy of `simple_tokenizer.py` (MIT): text cleaned and lowercased, split
//! into words, numbers and punctuation runs, each turned into byte symbols and merged pair by
//! pair in the order the merges file ranks them.

use std::collections::HashMap;

/// Start and end of text.
pub const START: i64 = 49406;
pub const END: i64 = 49407;

/// The merges CLIP uses from its file (after the header line).
const MERGES: usize = 49152 - 256 - 2;

pub struct ClipTokenizer {
    encoder: HashMap<String, i64>,
    ranks: HashMap<(String, String), usize>,
    /// Each byte as the printable character standing for it.
    bytes: [char; 256],
}

impl std::fmt::Debug for ClipTokenizer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClipTokenizer")
            .field("vocabulary", &self.encoder.len())
            .finish_non_exhaustive()
    }
}

/// CLIP's `bytes_to_unicode`: printable bytes stand for themselves; the others are shifted
/// past 255, in order. Also the order of the base vocabulary.
fn byte_symbols() -> ([char; 256], Vec<u8>) {
    let mut order: Vec<u8> = (b'!'..=b'~')
        .chain(0xa1..=0xac)
        .chain(0xae..=0xff)
        .collect();
    let mut chars = [' '; 256];
    for &b in &order {
        chars[b as usize] = char::from(b);
    }
    let mut n = 0;
    for b in 0..=255u8 {
        if !order.contains(&b) {
            order.push(b);
            // Invariant: 256 + n < 512, a valid scalar value.
            chars[b as usize] = char::from_u32(256 + n).expect("a valid character");
            n += 1;
        }
    }
    (chars, order)
}

impl ClipTokenizer {
    /// From the merges file's text (`bpe_simple_vocab_16e6.txt`, its first line a header).
    pub fn from_merges(text: &str) -> Result<Self, String> {
        let merges: Vec<(String, String)> = text
            .lines()
            .skip(1)
            .take(MERGES)
            .map(|line| {
                let mut parts = line.split_whitespace();
                match (parts.next(), parts.next()) {
                    (Some(a), Some(b)) => Ok((a.to_owned(), b.to_owned())),
                    _ => Err(format!("malformed merge {line:?}")),
                }
            })
            .collect::<Result<_, _>>()?;
        if merges.len() != MERGES {
            return Err(format!("{} merges, {MERGES} expected", merges.len()));
        }
        let (bytes, order) = byte_symbols();
        let mut vocab: Vec<String> = order
            .iter()
            .map(|&b| bytes[b as usize].to_string())
            .collect();
        vocab.extend(order.iter().map(|&b| format!("{}</w>", bytes[b as usize])));
        vocab.extend(merges.iter().map(|(a, b)| format!("{a}{b}")));
        vocab.push("<|startoftext|>".to_owned());
        vocab.push("<|endoftext|>".to_owned());
        let encoder = vocab
            .into_iter()
            .enumerate()
            .map(|(i, v)| (v, i as i64))
            .collect();
        let ranks = merges
            .into_iter()
            .enumerate()
            .map(|(i, pair)| (pair, i))
            .collect();
        Ok(Self {
            encoder,
            ranks,
            bytes,
        })
    }

    /// `text` as CLIP's ids, without start and end.
    pub fn encode(&self, text: &str) -> Vec<i64> {
        let clean = text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        let mut ids = Vec::new();
        for piece in split(&clean) {
            match piece {
                "<|startoftext|>" => ids.push(START),
                "<|endoftext|>" => ids.push(END),
                _ => {
                    let symbols: String = piece.bytes().map(|b| self.bytes[b as usize]).collect();
                    for token in self.bpe(&symbols) {
                        if let Some(&id) = self.encoder.get(&token) {
                            ids.push(id);
                        }
                    }
                }
            }
        }
        ids
    }

    /// `text` for a model with `context` tokens: start, ids, end, then zeros (too long: cut,
    /// the end token kept last).
    pub fn tokens(&self, text: &str, context: usize) -> Vec<i64> {
        let mut tokens = vec![START];
        tokens.extend(self.encode(text));
        tokens.push(END);
        if tokens.len() > context {
            tokens.truncate(context);
            if let Some(last) = tokens.last_mut() {
                *last = END;
            }
        }
        tokens.resize(context, 0);
        tokens
    }

    /// One word's byte symbols merged by rank: the symbols of its tokens.
    fn bpe(&self, word: &str) -> Vec<String> {
        let mut parts: Vec<String> = word.chars().map(String::from).collect();
        if let Some(last) = parts.last_mut() {
            last.push_str("</w>");
        }
        loop {
            let best = parts
                .windows(2)
                .filter_map(|pair| {
                    self.ranks
                        .get(&(pair[0].clone(), pair[1].clone()))
                        .map(|&rank| (rank, pair[0].clone(), pair[1].clone()))
                })
                .min_by_key(|(rank, _, _)| *rank);
            let Some((_, first, second)) = best else {
                return parts;
            };
            let mut merged = Vec::with_capacity(parts.len());
            let mut i = 0;
            while i < parts.len() {
                if i + 1 < parts.len() && parts[i] == first && parts[i + 1] == second {
                    merged.push(format!("{first}{second}"));
                    i += 2;
                } else {
                    merged.push(parts[i].clone());
                    i += 1;
                }
            }
            parts = merged;
        }
    }
}

/// The reference's pattern: the special tokens, English contractions, runs of word characters,
/// runs of other non-space characters.
fn split(text: &str) -> Vec<&str> {
    const SPECIAL: [&str; 2] = ["<|startoftext|>", "<|endoftext|>"];
    const CONTRACTIONS: [&str; 7] = ["'s", "'t", "'re", "'ve", "'m", "'ll", "'d"];
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let mut pieces = Vec::new();
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        let matched = SPECIAL
            .iter()
            .chain(CONTRACTIONS.iter())
            .find(|p| rest.starts_with(**p))
            .map(|p| p.len());
        let len = if let Some(len) = matched {
            len
        } else if c.is_whitespace() {
            rest = &rest[c.len_utf8()..];
            continue;
        } else if word(c) {
            rest.find(|ch: char| !word(ch)).unwrap_or(rest.len())
        } else {
            rest.find(|ch: char| ch.is_whitespace() || word(ch))
                .unwrap_or(rest.len())
        };
        pieces.push(&rest[..len]);
        rest = &rest[len..];
    }
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_like_the_reference() {
        assert_eq!(
            split("the cat's toy, 42 times!!"),
            ["the", "cat", "'s", "toy", ",", "42", "times", "!!"]
        );
    }

    /// With CLIP's real merges file (`SLOPSHOP_CLIP_MERGES`): known ids. Skipped otherwise.
    #[test]
    fn known_ids_with_the_real_merges() {
        let Some(path) = std::env::var_os("SLOPSHOP_CLIP_MERGES") else {
            eprintln!("SLOPSHOP_CLIP_MERGES not set: skipped");
            return;
        };
        let text = std::fs::read_to_string(path).unwrap();
        let tokenizer = ClipTokenizer::from_merges(&text).unwrap();
        // "a" 320, "photo" 1125, "of" 539, "cat" 2368 in CLIP's vocabulary.
        let tokens = tokenizer.tokens("A  photo of a CAT", 32);
        assert_eq!(&tokens[..7], [START, 320, 1125, 539, 320, 2368, END]);
        assert!(tokens[7..].iter().all(|&t| t == 0));
        assert_eq!(tokens.len(), 32);
    }
}
