//! A parser that turns PGN text into games.
//!
//! A PGN file may hold any number of games, so [`PgnParser::parse`] returns a
//! [`ParsedGame`] per game rather than a single move list. Each game carries its
//! moves and, when the file names it, a display name built from the tag pairs.
//!
//! The parser works under the assumption that the input describes legal games,
//! so it does not check whether a move is actually playable. It does, however,
//! reject text that is not well-formed SAN. This module depends only on the
//! `Action` types from the sibling `action` module.

use crate::action::Action;
use crate::action::ActionKind;
use crate::action::CheckIndicator;
use crate::action::PieceKind;
use crate::action::Square;

/// Errors that can occur while parsing PGN.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ParseError {
    /// A move token was empty after stripping annotations.
    #[error("empty move token")]
    EmptyMove,
    /// A token could not be understood as a SAN move.
    #[error("unrecognized move token: {0:?}")]
    UnrecognizedMove(String),
    /// A move token did not end in a valid destination square.
    #[error("missing or invalid destination square in move: {0:?}")]
    MissingDestination(String),
    /// A promotion target was missing or not a real piece.
    #[error("invalid promotion in move {token:?}: {piece:?}")]
    InvalidPromotion { token: String, piece: char },
    /// A tag pair was not of the form `[Name "value"]`.
    #[error("malformed tag pair: {0}")]
    MalformedTag(String),
}

/// One game read out of a PGN file.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct ParsedGame {
    /// The moves of the game's main line, in order.
    pub actions: Vec<Action>,
    /// A display name built from the game's tag pairs, or `None` when the file
    /// says nothing about the game. Callers decide what to show in that case;
    /// the parser only reports what the file actually contains.
    pub name: Option<String>,
}

/// A single lexical item of a PGN file.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Token<'a> {
    /// A tag pair such as `[White "Kasparov"]`. The value is unescaped, so it
    /// is owned rather than borrowed.
    Tag { name: &'a str, value: String },
    /// A SAN move, with any move number already stripped.
    Move(&'a str),
    /// A game terminator: `1-0`, `0-1`, `1/2-1/2`, or `*`.
    Result,
}

/// Parses PGN text into games.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct PgnParser;

impl PgnParser {
    #[must_use]
    pub fn new() -> Self {
        PgnParser
    }

    /// Parses a PGN string into its games, in file order.
    ///
    /// A new game starts at the first tag pair after a game's moves, and a
    /// result token (`1-0`, `0-1`, `1/2-1/2`, `*`) ends one, so files with and
    /// without tag pairs both split correctly. Comments (`{...}` and `;...`),
    /// numeric annotation glyphs (`$1`), recursive variations (`(...)`), and
    /// move numbers are ignored; everything else is read as a SAN move.
    ///
    /// # Errors
    ///
    /// Returns a [`ParseError`] if a tag pair is malformed, or if a token cannot
    /// be read as a SAN move: an empty move, an unrecognized token, a missing or
    /// malformed destination square, or an invalid promotion piece.
    pub fn parse(&self, pgn: &str) -> Result<Vec<ParsedGame>, ParseError> {
        let tokens = tokenize(pgn)?;
        split_games(tokens)
    }
}

// ===========================================================================
// TOKENIZING
// ===========================================================================

/// Splits PGN text into tag pairs, moves, and result tokens, discarding
/// comments, glyphs, and variations.
fn tokenize(pgn: &str) -> Result<Vec<Token<'_>>, ParseError> {
    let bytes = pgn.as_bytes();
    let mut tokens = Vec::new();
    let mut index = 0;

    while index < bytes.len() {
        match bytes[index] {
            b' ' | b'\t' | b'\r' | b'\n' => index += 1,
            // Comments are checked before tag pairs on purpose: a comment may
            // contain brackets, as Lichess's `{ [%clk 0:03:00] }` does, and
            // reading those as tag pairs would invent games.
            b'{' => skip_brace_comment(bytes, &mut index),
            b';' => skip_to_end_of_line(bytes, &mut index),
            b'(' => skip_variation(bytes, &mut index),
            b'$' => {
                index += 1;
                while index < bytes.len() && bytes[index].is_ascii_digit() {
                    index += 1;
                }
            }
            b'[' => {
                let (name, value) = read_tag(pgn, &mut index)?;
                tokens.push(Token::Tag { name, value });
            }
            // Stray closers. These are also word terminators, so if they were
            // not consumed here the word below would be empty and the index
            // would never advance. Kept separate from the whitespace arm above
            // even though the body matches: the two mean different things, and
            // merging them would couple an edit to one to the other.
            #[allow(clippy::match_same_arms)]
            b')' | b']' | b'}' => index += 1,
            _ => {
                let start = index;
                while index < bytes.len() && !is_word_end(bytes[index]) {
                    index += 1;
                }
                let word = &pgn[start..index];
                if is_result(word) {
                    tokens.push(Token::Result);
                } else {
                    let core = strip_leading_move_number(word);
                    // A bare move number leaves nothing behind.
                    if !core.is_empty() && !core.bytes().all(|byte| byte.is_ascii_digit()) {
                        tokens.push(Token::Move(core));
                    }
                }
            }
        }
    }
    Ok(tokens)
}

/// Bytes that end a whitespace-delimited word. Comments and variations may be
/// written flush against a move, so they terminate a word too.
fn is_word_end(byte: u8) -> bool {
    matches!(
        byte,
        b' ' | b'\t' | b'\r' | b'\n' | b'{' | b'}' | b'(' | b')' | b'[' | b']' | b';'
    )
}

fn skip_brace_comment(bytes: &[u8], index: &mut usize) {
    while *index < bytes.len() && bytes[*index] != b'}' {
        *index += 1;
    }
    // Step past the '}', or stop at the end for an unterminated comment.
    *index = (*index + 1).min(bytes.len());
}

fn skip_to_end_of_line(bytes: &[u8], index: &mut usize) {
    while *index < bytes.len() && bytes[*index] != b'\n' {
        *index += 1;
    }
}

/// Skips a recursive variation, including any nested variations and any
/// comments inside it (which may themselves contain parentheses).
fn skip_variation(bytes: &[u8], index: &mut usize) {
    let mut depth = 0usize;
    while *index < bytes.len() {
        match bytes[*index] {
            b'(' => {
                depth += 1;
                *index += 1;
            }
            b')' => {
                *index += 1;
                depth -= 1;
                if depth == 0 {
                    return;
                }
            }
            b'{' => skip_brace_comment(bytes, index),
            b';' => skip_to_end_of_line(bytes, index),
            _ => *index += 1,
        }
    }
}

/// Reads a `[Name "value"]` tag pair, honouring `\"` and `\\` escapes inside
/// the value so that a value containing a bracket cannot end the tag early.
fn read_tag<'a>(pgn: &'a str, index: &mut usize) -> Result<(&'a str, String), ParseError> {
    let bytes = pgn.as_bytes();
    *index += 1; // step past '['
    skip_spaces(bytes, index);

    let name_start = *index;
    while *index < bytes.len() && (bytes[*index].is_ascii_alphanumeric() || bytes[*index] == b'_') {
        *index += 1;
    }
    let name = &pgn[name_start..*index];
    if name.is_empty() {
        return Err(ParseError::MalformedTag("missing tag name".to_string()));
    }
    skip_spaces(bytes, index);

    if *index >= bytes.len() || bytes[*index] != b'"' {
        return Err(ParseError::MalformedTag(format!(
            "tag {name:?} has no quoted value"
        )));
    }
    *index += 1;

    let mut value = String::new();
    loop {
        if *index >= bytes.len() {
            return Err(ParseError::MalformedTag(format!(
                "tag {name:?} has an unterminated value"
            )));
        }
        match bytes[*index] {
            b'"' => {
                *index += 1;
                break;
            }
            b'\\' if *index + 1 < bytes.len() => {
                // Only `\"` and `\\` are escapes; anything else is literal.
                let escaped = bytes[*index + 1];
                if escaped == b'"' || escaped == b'\\' {
                    value.push(escaped as char);
                    *index += 2;
                } else {
                    value.push('\\');
                    *index += 1;
                }
            }
            _ => {
                let start = *index;
                // Advance one whole character to stay on a UTF-8 boundary.
                *index += 1;
                while *index < bytes.len() && (bytes[*index] & 0xC0) == 0x80 {
                    *index += 1;
                }
                value.push_str(&pgn[start..*index]);
            }
        }
    }

    skip_spaces(bytes, index);
    if *index >= bytes.len() || bytes[*index] != b']' {
        return Err(ParseError::MalformedTag(format!(
            "tag {name:?} is not closed with ']'"
        )));
    }
    *index += 1;
    Ok((name, value))
}

fn skip_spaces(bytes: &[u8], index: &mut usize) {
    while *index < bytes.len() && (bytes[*index] == b' ' || bytes[*index] == b'\t') {
        *index += 1;
    }
}

// ===========================================================================
// SPLITTING INTO GAMES
// ===========================================================================

/// Groups tokens into games. Tag pairs that follow a game's moves begin the next
/// game, and a result token ends the current one.
fn split_games(tokens: Vec<Token<'_>>) -> Result<Vec<ParsedGame>, ParseError> {
    let mut games = Vec::new();
    let mut tags: Vec<(String, String)> = Vec::new();
    let mut actions: Vec<Action> = Vec::new();
    let mut seen_moves = false;

    for token in tokens {
        match token {
            Token::Tag { name, value } => {
                if seen_moves {
                    games.push(finish_game(&mut tags, &mut actions));
                    seen_moves = false;
                }
                tags.push((name.to_string(), value));
            }
            Token::Move(word) => {
                actions.push(parse_san_move(word)?);
                seen_moves = true;
            }
            Token::Result => {
                // A stray result with nothing before it does not make a game.
                if seen_moves || !tags.is_empty() {
                    games.push(finish_game(&mut tags, &mut actions));
                    seen_moves = false;
                }
            }
        }
    }

    if !actions.is_empty() || !tags.is_empty() {
        games.push(finish_game(&mut tags, &mut actions));
    }
    Ok(games)
}

fn finish_game(tags: &mut Vec<(String, String)>, actions: &mut Vec<Action>) -> ParsedGame {
    let name = name_from_tags(tags);
    tags.clear();
    ParsedGame {
        actions: std::mem::take(actions),
        name,
    }
}

/// Looks up a tag, treating PGN's placeholders for unknown values (`?`, and
/// dates like `????.??.??`) as absent.
fn tag_value<'a>(tags: &'a [(String, String)], wanted: &str) -> Option<&'a str> {
    tags.iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(wanted))
        .map(|(_, value)| value.trim())
        .filter(|value| {
            !value.is_empty()
                && !value
                    .chars()
                    .all(|character| character == '?' || character == '.')
        })
}

/// Builds a display name from the tag pairs, using every part the file gives.
/// Returns `None` when the file names nothing at all.
fn name_from_tags(tags: &[(String, String)]) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();

    match (tag_value(tags, "White"), tag_value(tags, "Black")) {
        (Some(white), Some(black)) => parts.push(format!("{white} vs {black}")),
        (Some(player), None) | (None, Some(player)) => parts.push(player.to_string()),
        (None, None) => {}
    }
    for field in ["Event", "Site", "Date"] {
        if let Some(value) = tag_value(tags, field) {
            parts.push(value.to_string());
        }
    }
    if let Some(round) = tag_value(tags, "Round") {
        parts.push(format!("round {round}"));
    }
    // `*` means the result is unknown, so it says nothing worth showing.
    if let Some(result) = tag_value(tags, "Result").filter(|value| *value != "*") {
        parts.push(result.to_string());
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join(", "))
    }
}

/// Strips a leading move-number prefix such as `1.`, `12.`, or `1...` from a
/// token, returning the remainder (which may be empty for a bare number).
fn strip_leading_move_number(token: &str) -> &str {
    let bytes = token.as_bytes();
    let mut digits_end = 0;
    while digits_end < bytes.len() && bytes[digits_end].is_ascii_digit() {
        digits_end += 1;
    }
    if digits_end == 0 {
        return token;
    }
    let mut dots_end = digits_end;
    while dots_end < bytes.len() && bytes[dots_end] == b'.' {
        dots_end += 1;
    }
    if dots_end > digits_end {
        &token[dots_end..]
    } else {
        token
    }
}

fn is_result(token: &str) -> bool {
    matches!(token, "1-0" | "0-1" | "1/2-1/2" | "*")
}

// ===========================================================================
// SAN MOVES
// ===========================================================================

fn piece_from_letter(letter: char) -> Option<PieceKind> {
    match letter {
        'K' => Some(PieceKind::King),
        'Q' => Some(PieceKind::Queen),
        'R' => Some(PieceKind::Rook),
        'B' => Some(PieceKind::Bishop),
        'N' => Some(PieceKind::Knight),
        _ => None,
    }
}

/// Parses a single SAN move token (with move number and surrounding noise
/// already removed).
fn parse_san_move(token: &str) -> Result<Action, ParseError> {
    if token.is_empty() {
        return Err(ParseError::EmptyMove);
    }

    // Peel off trailing check, checkmate, and annotation glyphs.
    let mut check = CheckIndicator::None;
    let bytes = token.as_bytes();
    let mut end = bytes.len();
    while end > 0 {
        match bytes[end - 1] {
            b'#' => {
                check = CheckIndicator::Checkmate;
                end -= 1;
            }
            b'+' => {
                if check == CheckIndicator::None {
                    check = CheckIndicator::Check;
                }
                end -= 1;
            }
            b'!' | b'?' => end -= 1,
            _ => break,
        }
    }
    let mut core = &token[..end];
    // An en passant marker occasionally trails the move; the capture itself is
    // already encoded by the `x`, so the marker can simply be dropped.
    core = core.trim_end_matches("e.p.").trim_end_matches("e.p");

    // Castling, allowing the zero digit as an alternative to the letter O.
    let castling_form = core.to_ascii_uppercase().replace('0', "O");
    if castling_form == "O-O-O" {
        return Ok(Action::new(ActionKind::CastleQueenside, check));
    }
    if castling_form == "O-O" {
        return Ok(Action::new(ActionKind::CastleKingside, check));
    }

    parse_normal_move(core, check)
}

fn parse_normal_move(core: &str, check: CheckIndicator) -> Result<Action, ParseError> {
    let mut remainder = core;

    // Leading piece letter, or a pawn if there is none.
    let mut piece = PieceKind::Pawn;
    if let Some(first) = remainder.chars().next() {
        if let Some(found) = piece_from_letter(first) {
            piece = found;
            remainder = &remainder[1..];
        }
    }

    // Promotion, written either `=Q` or, more rarely, as a trailing piece letter.
    let mut promotion = None;
    if let Some(equals_index) = remainder.find('=') {
        let target = remainder[equals_index + 1..]
            .chars()
            .next()
            .ok_or_else(|| ParseError::InvalidPromotion {
                token: core.to_string(),
                piece: '=',
            })?;
        let promoted = piece_from_letter(target.to_ascii_uppercase()).ok_or_else(|| {
            ParseError::InvalidPromotion {
                token: core.to_string(),
                piece: target,
            }
        })?;
        promotion = Some(promoted);
        remainder = &remainder[..equals_index];
    } else if let Some(last) = remainder.chars().last() {
        if piece_from_letter(last).is_some() {
            promotion = piece_from_letter(last);
            remainder = &remainder[..remainder.len() - 1];
        }
    }

    // The destination is the trailing file+rank pair.
    let remainder_bytes = remainder.as_bytes();
    if remainder_bytes.len() < 2 {
        return Err(ParseError::MissingDestination(core.to_string()));
    }
    let file_byte = remainder_bytes[remainder_bytes.len() - 2];
    let rank_byte = remainder_bytes[remainder_bytes.len() - 1];
    if !(b'a'..=b'h').contains(&file_byte) || !(b'1'..=b'8').contains(&rank_byte) {
        return Err(ParseError::MissingDestination(core.to_string()));
    }
    let destination = Square::new(file_byte - b'a', rank_byte - b'1');

    // Anything before the destination is capture marker and/or disambiguation.
    let mut is_capture = false;
    let mut source_file = None;
    let mut source_rank = None;
    for character in remainder[..remainder.len() - 2].chars() {
        match character {
            'x' | 'X' => is_capture = true,
            'a'..='h' => source_file = Some(character as u8 - b'a'),
            '1'..='8' => source_rank = Some(character as u8 - b'1'),
            _ => return Err(ParseError::UnrecognizedMove(core.to_string())),
        }
    }

    Ok(Action::new(
        ActionKind::Normal {
            piece,
            source_file,
            source_rank,
            destination,
            is_capture,
            promotion,
        },
        check,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parses and asserts the reconstructed SAN of every move of every game.
    /// `Action`'s `Display` is unit tested in its own module, so matching the
    /// canonical SAN proves each move parsed into the correct components.
    fn assert_games(pgn: &str, expected: &[&[&str]]) {
        let games = PgnParser::new().parse(pgn).expect("pgn should parse");
        let actual: Vec<Vec<String>> = games
            .iter()
            .map(|game| game.actions.iter().map(ToString::to_string).collect())
            .collect();
        let actual: Vec<Vec<&str>> = actual
            .iter()
            .map(|game| game.iter().map(String::as_str).collect())
            .collect();
        let expected: Vec<Vec<&str>> = expected.iter().map(|game| game.to_vec()).collect();
        assert_eq!(actual, expected);
    }

    /// Convenience for the many single-game cases.
    fn assert_moves(pgn: &str, expected: &[&str]) {
        assert_games(pgn, &[expected]);
    }

    fn names(pgn: &str) -> Vec<Option<String>> {
        PgnParser::new()
            .parse(pgn)
            .expect("pgn should parse")
            .into_iter()
            .map(|game| game.name)
            .collect()
    }

    // ---- single games: movetext features ----

    #[test]
    fn fools_mate() {
        assert_moves("1. f3 e5 2. g4 Qh4#", &["f3", "e5", "g4", "Qh4#"]);
    }

    #[test]
    fn scholars_mate_with_annotations() {
        assert_moves(
            "1. e4 e5 2. Bc4 Nc6 3. Qh5 Nf6?? 4. Qxf7#",
            &["e4", "e5", "Bc4", "Nc6", "Qh5", "Nf6", "Qxf7#"],
        );
    }

    #[test]
    fn numbers_glued_to_moves() {
        assert_moves("1.e4 e5 2.Nf3 Nc6", &["e4", "e5", "Nf3", "Nc6"]);
    }

    #[test]
    fn black_move_number_continuation() {
        assert_moves("1. e4 1... e5 2. Nf3", &["e4", "e5", "Nf3"]);
    }

    #[test]
    fn headers_comments_and_nags_are_ignored() {
        let pgn = "[Event \"Test\"]\n[Site \"?\"]\n\n1. e4 {best by test} e5 2. Nf3 $1 Nc6 *";
        assert_moves(pgn, &["e4", "e5", "Nf3", "Nc6"]);
    }

    #[test]
    fn line_comments_are_ignored() {
        assert_moves("1. e4 ; this is a comment e5 Nf3\n2. d4", &["e4", "d4"]);
    }

    #[test]
    fn variations_are_ignored() {
        assert_moves(
            "1. e4 e5 2. Nf3 (2. f4 exf4 3. Bc4) Nc6 3. Bb5 a6",
            &["e4", "e5", "Nf3", "Nc6", "Bb5", "a6"],
        );
    }

    #[test]
    fn nested_variations_are_ignored() {
        assert_moves(
            "1. e4 e5 2. Nf3 (2. f4 exf4 (2... d5 3. exd5) 3. Bc4) Nc6",
            &["e4", "e5", "Nf3", "Nc6"],
        );
    }

    /// A comment inside a variation may itself contain a parenthesis; the
    /// variation must still end at the right place.
    #[test]
    fn variation_containing_a_comment_with_parentheses() {
        assert_moves(
            "1. e4 e5 2. Nf3 (2. f4 {the King's Gambit (sharp)} exf4) Nc6",
            &["e4", "e5", "Nf3", "Nc6"],
        );
    }

    #[test]
    fn comment_flush_against_a_move() {
        assert_moves("1. e4{a comment}e5", &["e4", "e5"]);
    }

    #[test]
    fn castling_with_zero_digits() {
        assert_moves(
            "1. O-O 0-0 2. O-O-O 0-0-0",
            &["O-O", "O-O", "O-O-O", "O-O-O"],
        );
    }

    #[test]
    fn promotions_including_capture_promotions() {
        assert_moves(
            "1. a4 h5 2. a5 h4 3. a6 h3 4. axb7 hxg2 5. bxa8=Q gxh1=Q",
            &[
                "a4", "h5", "a5", "h4", "a6", "h3", "axb7", "hxg2", "bxa8=Q", "gxh1=Q",
            ],
        );
    }

    #[test]
    fn en_passant_with_marker() {
        assert_moves(
            "1. e4 d5 2. e5 f5 3. exf6e.p.",
            &["e4", "d5", "e5", "f5", "exf6"],
        );
    }

    #[test]
    fn file_and_rank_disambiguation() {
        assert_moves(
            "1. Rae1 Nbd7 2. R1e2 N7f6",
            &["Rae1", "Nbd7", "R1e2", "N7f6"],
        );
    }

    #[test]
    fn full_square_disambiguation() {
        assert_moves("1. Qa1e5 Na1b3", &["Qa1e5", "Na1b3"]);
    }

    // ---- multiple games ----

    #[test]
    fn two_games_with_headers() {
        let pgn = "\
[Event \"Game one\"]
[Result \"1-0\"]

1. e4 e5 2. Bc4 Nc6 3. Qh5 Nf6 4. Qxf7# 1-0

[Event \"Game two\"]
[Result \"0-1\"]

1. f3 e5 2. g4 Qh4# 0-1
";
        assert_games(
            pgn,
            &[
                &["e4", "e5", "Bc4", "Nc6", "Qh5", "Nf6", "Qxf7#"],
                &["f3", "e5", "g4", "Qh4#"],
            ],
        );
    }

    /// Without tag pairs the only boundary is the result token.
    #[test]
    fn two_games_without_headers_split_on_results() {
        assert_games("1. e4 e5 1-0 1. d4 d5 0-1", &[&["e4", "e5"], &["d4", "d5"]]);
    }

    /// A game may follow the previous one's moves with no result in between;
    /// the next tag pair starts it.
    #[test]
    fn tag_pair_after_moves_starts_a_new_game() {
        let pgn = "[Event \"A\"] 1. e4 e5 [Event \"B\"] 1. d4 d5";
        assert_games(pgn, &[&["e4", "e5"], &["d4", "d5"]]);
        assert_eq!(
            names(pgn),
            vec![Some("A".to_string()), Some("B".to_string())]
        );
    }

    #[test]
    fn a_final_game_without_a_result_is_still_a_game() {
        assert_games("1. e4 e5 1-0 1. d4 d5", &[&["e4", "e5"], &["d4", "d5"]]);
    }

    #[test]
    fn three_games() {
        let pgn = "1. e4 1-0 1. d4 0-1 1. c4 1/2-1/2";
        assert_games(pgn, &[&["e4"], &["d4"], &["c4"]]);
    }

    #[test]
    fn empty_input_yields_no_games() {
        assert_eq!(PgnParser::new().parse("").unwrap(), Vec::new());
        assert_eq!(PgnParser::new().parse("   \n\t ").unwrap(), Vec::new());
    }

    /// A lone result token describes nothing and must not invent a game.
    #[test]
    fn stray_result_is_not_a_game() {
        assert_eq!(PgnParser::new().parse("*").unwrap(), Vec::new());
        assert_eq!(PgnParser::new().parse("1-0 0-1").unwrap(), Vec::new());
    }

    /// Headers with no moves still describe a game, just an empty one.
    #[test]
    fn header_only_game_has_no_moves() {
        let games = PgnParser::new().parse("[Event \"Empty\"] *").unwrap();
        assert_eq!(games.len(), 1);
        assert!(games[0].actions.is_empty());
        assert_eq!(games[0].name, Some("Empty".to_string()));
    }

    // ---- names ----

    #[test]
    fn name_uses_every_part_the_file_gives() {
        let pgn = "\
[Event \"Wijk aan Zee\"]
[Site \"NED\"]
[Date \"1999.01.20\"]
[Round \"4\"]
[White \"Kasparov\"]
[Black \"Topalov\"]
[Result \"1-0\"]

1. e4 d6 1-0
";
        assert_eq!(
            names(pgn),
            vec![Some(
                "Kasparov vs Topalov, Wijk aan Zee, NED, 1999.01.20, round 4, 1-0".to_string()
            )]
        );
    }

    #[test]
    fn name_from_players_only() {
        let pgn = "[White \"Alice\"] [Black \"Bob\"] 1. e4 *";
        assert_eq!(names(pgn), vec![Some("Alice vs Bob".to_string())]);
    }

    #[test]
    fn name_with_only_one_player_named() {
        let pgn = "[White \"Alice\"] 1. e4 *";
        assert_eq!(names(pgn), vec![Some("Alice".to_string())]);
    }

    /// PGN writes unknown values as `?` and unknown dates as `????.??.??`.
    #[test]
    fn unknown_placeholders_are_left_out_of_the_name() {
        let pgn = "\
[Event \"?\"]
[Site \"?\"]
[Date \"????.??.??\"]
[Round \"?\"]
[White \"Alice\"]
[Black \"Bob\"]
[Result \"*\"]

1. e4 *
";
        assert_eq!(names(pgn), vec![Some("Alice vs Bob".to_string())]);
    }

    #[test]
    fn a_game_without_tags_has_no_name() {
        assert_eq!(names("1. e4 e5 1-0"), vec![None]);
    }

    #[test]
    fn names_are_per_game() {
        let pgn = "[White \"Alice\"] 1. e4 1-0 1. d4 0-1 [White \"Bob\"] 1. c4 1/2-1/2";
        assert_eq!(
            names(pgn),
            vec![Some("Alice".to_string()), None, Some("Bob".to_string())]
        );
    }

    #[test]
    fn unresolved_result_is_left_out_of_the_name() {
        let pgn = "[Event \"Ongoing\"] [Result \"*\"] 1. e4 *";
        assert_eq!(names(pgn), vec![Some("Ongoing".to_string())]);
    }

    // ---- tag parsing corner cases ----

    /// Lichess writes clock annotations inside comments. Those brackets must not
    /// be read as tag pairs, or every move would start a new game.
    #[test]
    fn bracket_inside_a_comment_is_not_a_tag() {
        let pgn = "[Event \"Rated\"] 1. e4 { [%clk 0:03:00] } e5 { [%clk 0:02:58] } 1-0";
        assert_games(pgn, &[&["e4", "e5"]]);
        assert_eq!(names(pgn), vec![Some("Rated".to_string())]);
    }

    #[test]
    fn tag_value_may_contain_a_closing_bracket() {
        let pgn = "[Event \"Foo] Bar\"] 1. e4 *";
        assert_eq!(names(pgn), vec![Some("Foo] Bar".to_string())]);
    }

    #[test]
    fn tag_value_may_contain_escaped_quotes_and_backslashes() {
        let pgn = "[Event \"He said \\\"hi\\\" C:\\\\games\"] 1. e4 *";
        assert_eq!(
            names(pgn),
            vec![Some("He said \"hi\" C:\\games".to_string())]
        );
    }

    #[test]
    fn tag_value_may_contain_non_ascii() {
        let pgn = "[White \"Réti\"] [Black \"Tartakower\"] 1. e4 *";
        assert_eq!(names(pgn), vec![Some("Réti vs Tartakower".to_string())]);
    }

    #[test]
    fn tag_names_are_matched_case_insensitively() {
        let pgn = "[white \"Alice\"] [BLACK \"Bob\"] 1. e4 *";
        assert_eq!(names(pgn), vec![Some("Alice vs Bob".to_string())]);
    }

    #[test]
    fn unknown_tags_do_not_break_parsing() {
        let pgn = "[ECO \"B90\"] [WhiteElo \"2800\"] [White \"Alice\"] 1. e4 *";
        assert_eq!(names(pgn), vec![Some("Alice".to_string())]);
    }

    #[test]
    fn malformed_tags_are_rejected() {
        let parser = PgnParser::new();
        for pgn in [
            "[Event no quotes] 1. e4",
            "[Event \"unterminated 1. e4",
            "[\"no name\"] 1. e4",
            "[Event \"value\" 1. e4",
        ] {
            assert!(
                matches!(parser.parse(pgn), Err(ParseError::MalformedTag(_))),
                "expected a MalformedTag error for {pgn:?}"
            );
        }
    }

    /// A stray closer is also a word terminator, so an early version of the
    /// tokenizer read an empty word and never advanced — the parser hung
    /// forever rather than returning. Malformed input must always terminate.
    #[test]
    fn stray_closers_terminate() {
        let parser = PgnParser::new();
        for pgn in [
            "1. e4 } e5",
            "}",
            "}}}",
            "]",
            ")",
            "( { ) } )",
            "1. e4 ] e5",
        ] {
            let _ = parser.parse(pgn);
        }
        // The moves either side of a stray brace are still read.
        assert_moves("1. e4 } e5", &["e4", "e5"]);
    }

    /// Unterminated constructs must stop at the end of input, not run past it.
    #[test]
    fn unterminated_constructs_terminate() {
        let parser = PgnParser::new();
        for pgn in ["1. e4 {unterminated", "1. e4 (", "((((", "[", "{", "$"] {
            let _ = parser.parse(pgn);
        }
    }

    // ---- move errors ----

    #[test]
    fn garbage_token_is_rejected() {
        assert!(PgnParser::new().parse("1. e4 Zx9").is_err());
    }

    #[test]
    fn invalid_promotion_is_rejected() {
        assert!(matches!(
            PgnParser::new().parse("1. e8=Z"),
            Err(ParseError::InvalidPromotion { .. })
        ));
    }

    #[test]
    fn missing_destination_is_rejected() {
        assert!(matches!(
            PgnParser::new().parse("1. Nx"),
            Err(ParseError::MissingDestination(_))
        ));
    }

    // ---- full games ----

    #[test]
    fn opera_game_morphy() {
        let pgn = "1. e4 e5 2. Nf3 d6 3. d4 Bg4 4. dxe5 Bxf3 5. Qxf3 dxe5 6. Bc4 Nf6 \
                   7. Qb3 Qe7 8. Nc3 c6 9. Bg5 b5 10. Nxb5 cxb5 11. Bxb5+ Nbd7 \
                   12. O-O-O Rd8 13. Rxd7 Rxd7 14. Rd1 Qe6 15. Bxd7+ Nxd7 \
                   16. Qb8+ Nxb8 17. Rd8# 1-0";
        assert_moves(
            pgn,
            &[
                "e4", "e5", "Nf3", "d6", "d4", "Bg4", "dxe5", "Bxf3", "Qxf3", "dxe5", "Bc4", "Nf6",
                "Qb3", "Qe7", "Nc3", "c6", "Bg5", "b5", "Nxb5", "cxb5", "Bxb5+", "Nbd7", "O-O-O",
                "Rd8", "Rxd7", "Rxd7", "Rd1", "Qe6", "Bxd7+", "Nxd7", "Qb8+", "Nxb8", "Rd8#",
            ],
        );
    }

    /// A realistic export: several games, headers, comments, glyphs, variations.
    #[test]
    fn realistic_multi_game_export() {
        let pgn = "\
[Event \"Wijk aan Zee\"]
[White \"Kasparov\"]
[Black \"Topalov\"]
[Result \"1-0\"]

1. e4 d6 {the Pirc} 2. d4 Nf6 $1 (2... g6 3. Nc3) 3. Nc3 g6 1-0

[Event \"London\"]
[White \"Lasker\"]
[Black \"Thomas\"]
[Result \"1-0\"]

1. d4 e6 ; a line comment
2. Nf3 f5 1-0
";
        assert_games(
            pgn,
            &[
                &["e4", "d6", "d4", "Nf6", "Nc3", "g6"],
                &["d4", "e6", "Nf3", "f5"],
            ],
        );
        assert_eq!(
            names(pgn),
            vec![
                Some("Kasparov vs Topalov, Wijk aan Zee, 1-0".to_string()),
                Some("Lasker vs Thomas, London, 1-0".to_string()),
            ]
        );
    }
}
