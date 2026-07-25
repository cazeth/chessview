//! The core chess game state.
//!
//! [`GameState`] wraps a tudi [`Grid`] of [`Piece`] and remembers whose turn it
//! is. It can be built from the standard starting position or from an ascii
//! board, and advanced one [`Action`] at a time with [`GameState::process_move`].
//!
//! The state assumes it is fed a legal game: it resolves each SAN move to the
//! piece that plays it (using geometry, the SAN disambiguation hints, and, when
//! two pieces of the same type could reach the square, a check on which one may
//! legally move) but it does not otherwise verify that a move is sound. It
//! understands captures, en passant, castling, and promotion. Its only project
//! dependency is the `action` module.

use crate::action::Action;
use crate::action::ActionKind;
use crate::action::PieceKind;
use crate::action::Square;
use tudi::Grid;

/// The side to move.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Color {
    White,
    Black,
}

impl Color {
    fn opponent(self) -> Color {
        match self {
            Color::White => Color::Black,
            Color::Black => Color::White,
        }
    }

    /// The rank direction pawns of this color advance in (in rank-index terms).
    fn pawn_direction(self) -> i8 {
        match self {
            Color::White => 1,
            Color::Black => -1,
        }
    }

    /// The back-rank index for this color (where the king and rooks start).
    fn back_rank(self) -> usize {
        match self {
            Color::White => 0,
            Color::Black => 7,
        }
    }
}

/// A colored chess piece. This is the element type stored in the [`Grid`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Piece {
    WhitePawn,
    WhiteKnight,
    WhiteBishop,
    WhiteRook,
    WhiteQueen,
    WhiteKing,
    BlackPawn,
    BlackKnight,
    BlackBishop,
    BlackRook,
    BlackQueen,
    BlackKing,
}

impl Piece {
    pub fn new(color: Color, kind: PieceKind) -> Self {
        match (color, kind) {
            (Color::White, PieceKind::Pawn) => Piece::WhitePawn,
            (Color::White, PieceKind::Knight) => Piece::WhiteKnight,
            (Color::White, PieceKind::Bishop) => Piece::WhiteBishop,
            (Color::White, PieceKind::Rook) => Piece::WhiteRook,
            (Color::White, PieceKind::Queen) => Piece::WhiteQueen,
            (Color::White, PieceKind::King) => Piece::WhiteKing,
            (Color::Black, PieceKind::Pawn) => Piece::BlackPawn,
            (Color::Black, PieceKind::Knight) => Piece::BlackKnight,
            (Color::Black, PieceKind::Bishop) => Piece::BlackBishop,
            (Color::Black, PieceKind::Rook) => Piece::BlackRook,
            (Color::Black, PieceKind::Queen) => Piece::BlackQueen,
            (Color::Black, PieceKind::King) => Piece::BlackKing,
        }
    }

    pub fn color(self) -> Color {
        match self {
            Piece::WhitePawn
            | Piece::WhiteKnight
            | Piece::WhiteBishop
            | Piece::WhiteRook
            | Piece::WhiteQueen
            | Piece::WhiteKing => Color::White,
            Piece::BlackPawn
            | Piece::BlackKnight
            | Piece::BlackBishop
            | Piece::BlackRook
            | Piece::BlackQueen
            | Piece::BlackKing => Color::Black,
        }
    }

    pub fn kind(self) -> PieceKind {
        match self {
            Piece::WhitePawn | Piece::BlackPawn => PieceKind::Pawn,
            Piece::WhiteKnight | Piece::BlackKnight => PieceKind::Knight,
            Piece::WhiteBishop | Piece::BlackBishop => PieceKind::Bishop,
            Piece::WhiteRook | Piece::BlackRook => PieceKind::Rook,
            Piece::WhiteQueen | Piece::BlackQueen => PieceKind::Queen,
            Piece::WhiteKing | Piece::BlackKing => PieceKind::King,
        }
    }

    /// The ascii character for this piece: uppercase for white, lowercase for
    /// black.
    pub fn to_char(self) -> char {
        let letter = match self.kind() {
            PieceKind::Pawn => 'P',
            PieceKind::Knight => 'N',
            PieceKind::Bishop => 'B',
            PieceKind::Rook => 'R',
            PieceKind::Queen => 'Q',
            PieceKind::King => 'K',
        };
        match self.color() {
            Color::White => letter,
            Color::Black => letter.to_ascii_lowercase(),
        }
    }

    /// Parses a single ascii board character into a piece, or `None` for a dot
    /// (empty square).
    pub fn from_char(character: char) -> Option<Piece> {
        let color = if character.is_ascii_uppercase() {
            Color::White
        } else {
            Color::Black
        };
        let kind = match character.to_ascii_uppercase() {
            'P' => PieceKind::Pawn,
            'N' => PieceKind::Knight,
            'B' => PieceKind::Bishop,
            'R' => PieceKind::Rook,
            'Q' => PieceKind::Queen,
            'K' => PieceKind::King,
            _ => return None,
        };
        Some(Piece::new(color, kind))
    }
}

/// Errors that can arise while building or advancing a game state.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum GameStateError {
    /// The ascii board could not be parsed.
    #[error("invalid board: {0}")]
    InvalidBoard(String),
    /// No piece of the moving side could legally reach the destination.
    #[error("no piece can play {0}")]
    NoPieceForMove(String),
    /// More than one piece could play the move and it could not be resolved.
    #[error("ambiguous move: {0}")]
    AmbiguousMove(String),
    /// An underlying grid operation failed.
    #[error("grid error: {0}")]
    Grid(String),
}

/// A plain 8x8 snapshot of the board, indexed `[rank][file]`, used for move
/// resolution and check detection. Being an array of a `Copy` type it is itself
/// `Copy`, so trial positions are free to make.
type Board = [[Option<Piece>; 8]; 8];

const KNIGHT_OFFSETS: [(i8, i8); 8] = [
    (1, 2),
    (2, 1),
    (2, -1),
    (1, -2),
    (-1, -2),
    (-2, -1),
    (-2, 1),
    (-1, 2),
];

const KING_OFFSETS: [(i8, i8); 8] = [
    (1, 0),
    (-1, 0),
    (0, 1),
    (0, -1),
    (1, 1),
    (1, -1),
    (-1, 1),
    (-1, -1),
];

const BISHOP_DIRECTIONS: [(i8, i8); 4] = [(1, 1), (1, -1), (-1, 1), (-1, -1)];
const ROOK_DIRECTIONS: [(i8, i8); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];

const STARTING_POSITION: &str = "\
rnbqkbnr
pppppppp
........
........
........
........
PPPPPPPP
RNBQKBNR";

/// The state of a chess game: the board and whose turn it is.
#[derive(Debug)]
pub struct GameState {
    board: Grid<Piece>,
    side_to_move: Color,
}

impl GameState {
    /// Builds the standard starting position with white to move.
    ///
    /// # Panics
    ///
    /// Panics only if the hardcoded starting position fails to parse, which
    /// would be a bug in this module rather than a fault of the caller.
    pub fn new() -> Self {
        Self::from_ascii(STARTING_POSITION).expect("the starting position is valid")
    }

    /// Builds a game state from an ascii board: eight rows, top row is rank 8,
    /// uppercase letters are white pieces, lowercase are black, and a dot is an
    /// empty square. White is set to move.
    ///
    /// # Errors
    ///
    /// Returns [`GameStateError::InvalidBoard`] if the text is not eight rows of
    /// eight characters, or holds a character that is neither a piece nor a dot,
    /// and [`GameStateError::Grid`] if a square cannot be written.
    pub fn from_ascii(text: &str) -> Result<Self, GameStateError> {
        let rows: Vec<&str> = text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect();
        if rows.len() != 8 {
            return Err(GameStateError::InvalidBoard(format!(
                "expected 8 rows, found {}",
                rows.len()
            )));
        }
        let mut board: Grid<Piece> = Grid::new(8, 8);
        for (row_index, row) in rows.iter().enumerate() {
            let characters: Vec<char> = row.chars().collect();
            if characters.len() != 8 {
                return Err(GameStateError::InvalidBoard(format!(
                    "row {} has {} columns, expected 8",
                    row_index,
                    characters.len()
                )));
            }
            let rank = 7 - row_index;
            for (file, &character) in characters.iter().enumerate() {
                if character == '.' {
                    continue;
                }
                let piece = Piece::from_char(character).ok_or_else(|| {
                    GameStateError::InvalidBoard(format!("unknown piece {character:?}"))
                })?;
                board
                    .store_element(&Square::new(file as u8, rank as u8), piece)
                    .map_err(|error| GameStateError::Grid(error.to_string()))?;
            }
        }
        Ok(GameState {
            board,
            side_to_move: Color::White,
        })
    }

    /// Renders the board as ascii in the same format `from_ascii` reads.
    pub fn to_ascii(&self) -> String {
        let mut output = String::new();
        for rank in (0..8).rev() {
            for file in 0..8 {
                let character = self
                    .board
                    .element_unchecked(&Square::new(file, rank))
                    .map_or('.', |piece| piece.to_char());
                output.push(character);
            }
            if rank != 0 {
                output.push('\n');
            }
        }
        output
    }

    /// Advances the game by one move.
    ///
    /// # Errors
    ///
    /// Returns [`GameStateError::NoPieceForMove`] if no piece of the moving side
    /// can reach the destination, [`GameStateError::AmbiguousMove`] if more than
    /// one could and the notation does not say which, and
    /// [`GameStateError::Grid`] if a square cannot be read or written.
    pub fn process_move(&mut self, action: Action) -> Result<(), GameStateError> {
        let color = self.side_to_move;
        match action.kind {
            ActionKind::CastleKingside => self.apply_castle(color, true)?,
            ActionKind::CastleQueenside => self.apply_castle(color, false)?,
            ActionKind::Normal {
                piece,
                source_file,
                source_rank,
                destination,
                is_capture,
                promotion,
            } => {
                let board = self.snapshot();
                let destination_file = destination.file() as usize;
                let destination_rank = destination.rank() as usize;
                let (origin_file, origin_rank) = resolve_origin(
                    &board,
                    color,
                    piece,
                    destination_file,
                    destination_rank,
                    is_capture,
                    source_file,
                    source_rank,
                    &action,
                )?;

                let is_en_passant = is_en_passant_capture(
                    &board,
                    piece,
                    is_capture,
                    destination_file,
                    destination_rank,
                );

                self.remove(origin_file, origin_rank)?;
                if is_en_passant {
                    // The captured pawn sits on the destination file at the
                    // rank the capturing pawn started from.
                    self.remove(destination_file, origin_rank)?;
                }
                let placed = match promotion {
                    Some(promoted_kind) => Piece::new(color, promoted_kind),
                    None => Piece::new(color, piece),
                };
                self.store(destination_file, destination_rank, placed)?;
            }
        }
        self.side_to_move = color.opponent();
        Ok(())
    }

    fn apply_castle(&mut self, color: Color, kingside: bool) -> Result<(), GameStateError> {
        let rank = color.back_rank();
        let (king_to, rook_from, rook_to) = if kingside { (6, 7, 5) } else { (2, 0, 3) };
        let king = self.remove(4, rank)?;
        let rook = self.remove(rook_from, rank)?;
        self.store(king_to, rank, king)?;
        self.store(rook_to, rank, rook)?;
        Ok(())
    }

    fn snapshot(&self) -> Board {
        let mut board = [[None; 8]; 8];
        for rank in 0..8 {
            for file in 0..8 {
                board[rank][file] = self
                    .board
                    .element_unchecked(&Square::new(file as u8, rank as u8))
                    .copied();
            }
        }
        board
    }

    fn remove(&mut self, file: usize, rank: usize) -> Result<Piece, GameStateError> {
        self.board
            .remove_element(&Square::new(file as u8, rank as u8))
            .map_err(|error| GameStateError::Grid(error.to_string()))
    }

    fn store(&mut self, file: usize, rank: usize, piece: Piece) -> Result<(), GameStateError> {
        self.board
            .store_element(&Square::new(file as u8, rank as u8), piece)
            .map(|_| ())
            .map_err(|error| GameStateError::Grid(error.to_string()))
    }
}

impl Default for GameState {
    fn default() -> Self {
        Self::new()
    }
}

/// Reads a square from the snapshot with bounds checking.
fn get(board: &Board, file: i8, rank: i8) -> Option<Piece> {
    if (0..8).contains(&file) && (0..8).contains(&rank) {
        board[rank as usize][file as usize]
    } else {
        None
    }
}

/// Whether a move is an en passant capture: a pawn capture onto an empty
/// square. The captured pawn is not on the destination but beside the moving
/// pawn, so callers that see this handle the removal specially. `board` is the
/// position before the move is placed.
fn is_en_passant_capture(
    board: &Board,
    piece: PieceKind,
    is_capture: bool,
    destination_file: usize,
    destination_rank: usize,
) -> bool {
    piece == PieceKind::Pawn && is_capture && board[destination_rank][destination_file].is_none()
}

#[allow(clippy::too_many_arguments)]
fn resolve_origin(
    board: &Board,
    color: Color,
    kind: PieceKind,
    destination_file: usize,
    destination_rank: usize,
    is_capture: bool,
    source_file: Option<u8>,
    source_rank: Option<u8>,
    action: &Action,
) -> Result<(usize, usize), GameStateError> {
    let mut candidates = candidate_origins(
        board,
        color,
        kind,
        destination_file,
        destination_rank,
        is_capture,
    );

    candidates.retain(|&(file, rank)| {
        source_file.is_none_or(|wanted| wanted as usize == file)
            && source_rank.is_none_or(|wanted| wanted as usize == rank)
    });

    if candidates.len() > 1 {
        candidates.retain(|&(file, rank)| {
            !move_leaves_king_in_check(
                board,
                color,
                kind,
                file,
                rank,
                destination_file,
                destination_rank,
                is_capture,
            )
        });
    }

    match candidates.len() {
        1 => Ok(candidates[0]),
        0 => Err(GameStateError::NoPieceForMove(action.to_string())),
        _ => Err(GameStateError::AmbiguousMove(action.to_string())),
    }
}

fn candidate_origins(
    board: &Board,
    color: Color,
    kind: PieceKind,
    destination_file: usize,
    destination_rank: usize,
    is_capture: bool,
) -> Vec<(usize, usize)> {
    match kind {
        PieceKind::Pawn => {
            pawn_origins(board, color, destination_file, destination_rank, is_capture)
        }
        PieceKind::Knight => offset_origins(
            board,
            color,
            kind,
            destination_file,
            destination_rank,
            &KNIGHT_OFFSETS,
        ),
        PieceKind::King => offset_origins(
            board,
            color,
            kind,
            destination_file,
            destination_rank,
            &KING_OFFSETS,
        ),
        PieceKind::Bishop => slider_origins(
            board,
            color,
            kind,
            destination_file,
            destination_rank,
            &BISHOP_DIRECTIONS,
        ),
        PieceKind::Rook => slider_origins(
            board,
            color,
            kind,
            destination_file,
            destination_rank,
            &ROOK_DIRECTIONS,
        ),
        PieceKind::Queen => {
            let mut origins = slider_origins(
                board,
                color,
                kind,
                destination_file,
                destination_rank,
                &BISHOP_DIRECTIONS,
            );
            origins.extend(slider_origins(
                board,
                color,
                kind,
                destination_file,
                destination_rank,
                &ROOK_DIRECTIONS,
            ));
            origins
        }
    }
}

fn pawn_origins(
    board: &Board,
    color: Color,
    destination_file: usize,
    destination_rank: usize,
    is_capture: bool,
) -> Vec<(usize, usize)> {
    let direction = color.pawn_direction();
    let our_pawn = Piece::new(color, PieceKind::Pawn);
    let destination_file = destination_file as i8;
    let destination_rank = destination_rank as i8;
    let mut origins = Vec::new();

    if is_capture {
        for file_offset in [-1i8, 1] {
            let origin_file = destination_file + file_offset;
            let origin_rank = destination_rank - direction;
            if get(board, origin_file, origin_rank) == Some(our_pawn) {
                origins.push((origin_file as usize, origin_rank as usize));
            }
        }
    } else {
        let single = destination_rank - direction;
        if get(board, destination_file, single) == Some(our_pawn) {
            origins.push((destination_file as usize, single as usize));
        } else {
            let double = destination_rank - 2 * direction;
            let intermediate_empty = get(board, destination_file, single).is_none();
            if intermediate_empty && get(board, destination_file, double) == Some(our_pawn) {
                origins.push((destination_file as usize, double as usize));
            }
        }
    }
    origins
}

fn offset_origins(
    board: &Board,
    color: Color,
    kind: PieceKind,
    destination_file: usize,
    destination_rank: usize,
    offsets: &[(i8, i8)],
) -> Vec<(usize, usize)> {
    let wanted = Piece::new(color, kind);
    let mut origins = Vec::new();
    for &(file_offset, rank_offset) in offsets {
        let file = destination_file as i8 + file_offset;
        let rank = destination_rank as i8 + rank_offset;
        if get(board, file, rank) == Some(wanted) {
            origins.push((file as usize, rank as usize));
        }
    }
    origins
}

fn slider_origins(
    board: &Board,
    color: Color,
    kind: PieceKind,
    destination_file: usize,
    destination_rank: usize,
    directions: &[(i8, i8)],
) -> Vec<(usize, usize)> {
    let wanted = Piece::new(color, kind);
    let mut origins = Vec::new();
    for &(file_step, rank_step) in directions {
        let mut file = destination_file as i8 + file_step;
        let mut rank = destination_rank as i8 + rank_step;
        while (0..8).contains(&file) && (0..8).contains(&rank) {
            if let Some(piece) = board[rank as usize][file as usize] {
                if piece == wanted {
                    origins.push((file as usize, rank as usize));
                }
                break;
            }
            file += file_step;
            rank += rank_step;
        }
    }
    origins
}

#[allow(clippy::too_many_arguments)]
fn move_leaves_king_in_check(
    board: &Board,
    color: Color,
    kind: PieceKind,
    origin_file: usize,
    origin_rank: usize,
    destination_file: usize,
    destination_rank: usize,
    is_capture: bool,
) -> bool {
    let mut trial = *board;
    let moving = trial[origin_rank][origin_file];
    trial[origin_rank][origin_file] = None;
    if is_en_passant_capture(&trial, kind, is_capture, destination_file, destination_rank) {
        trial[origin_rank][destination_file] = None;
    }
    trial[destination_rank][destination_file] = moving;

    match find_king(&trial, color) {
        Some((king_file, king_rank)) => is_attacked(&trial, king_file, king_rank, color.opponent()),
        None => false,
    }
}

fn find_king(board: &Board, color: Color) -> Option<(usize, usize)> {
    let king = Piece::new(color, PieceKind::King);
    for rank in 0..8 {
        for file in 0..8 {
            if board[rank][file] == Some(king) {
                return Some((file, rank));
            }
        }
    }
    None
}

fn is_attacked(board: &Board, file: usize, rank: usize, by: Color) -> bool {
    let file = file as i8;
    let rank = rank as i8;

    let pawn_direction = by.pawn_direction();
    for file_offset in [-1i8, 1] {
        if get(board, file + file_offset, rank - pawn_direction)
            == Some(Piece::new(by, PieceKind::Pawn))
        {
            return true;
        }
    }

    for &(file_offset, rank_offset) in &KNIGHT_OFFSETS {
        if get(board, file + file_offset, rank + rank_offset)
            == Some(Piece::new(by, PieceKind::Knight))
        {
            return true;
        }
    }

    for &(file_offset, rank_offset) in &KING_OFFSETS {
        if get(board, file + file_offset, rank + rank_offset)
            == Some(Piece::new(by, PieceKind::King))
        {
            return true;
        }
    }

    if slider_attacks(board, file, rank, by, &BISHOP_DIRECTIONS, PieceKind::Bishop) {
        return true;
    }
    if slider_attacks(board, file, rank, by, &ROOK_DIRECTIONS, PieceKind::Rook) {
        return true;
    }
    false
}

fn slider_attacks(
    board: &Board,
    file: i8,
    rank: i8,
    by: Color,
    directions: &[(i8, i8)],
    straight_kind: PieceKind,
) -> bool {
    let queen = Piece::new(by, PieceKind::Queen);
    let straight = Piece::new(by, straight_kind);
    for &(file_step, rank_step) in directions {
        let mut current_file = file + file_step;
        let mut current_rank = rank + rank_step;
        while (0..8).contains(&current_file) && (0..8).contains(&current_rank) {
            if let Some(piece) = board[current_rank as usize][current_file as usize] {
                if piece == queen || piece == straight {
                    return true;
                }
                break;
            }
            current_file += file_step;
            current_rank += rank_step;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::PgnParser;

    /// Trims indentation so board literals can be written inline.
    fn normalize(board: &str) -> String {
        board
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Plays a full game from the starting position and returns the final board.
    fn play_game(pgn: &str) -> String {
        let games = PgnParser::new().parse(pgn).expect("pgn should parse");
        let actions = games.into_iter().next().expect("one game").actions;
        let mut state = GameState::new();
        for action in actions {
            state.process_move(action).expect("move should apply");
        }
        state.to_ascii()
    }

    /// Sets up a board, plays the given movetext, and returns the final board.
    fn play_from(board: &str, movetext: &str) -> String {
        let games = PgnParser::new()
            .parse(movetext)
            .expect("movetext should parse");
        let actions = games.into_iter().next().expect("one game").actions;
        let mut state = GameState::from_ascii(board).expect("board should parse");
        for action in actions {
            state.process_move(action).expect("move should apply");
        }
        state.to_ascii()
    }

    #[test]
    fn starting_position_round_trips() {
        assert_eq!(GameState::new().to_ascii(), normalize(STARTING_POSITION));
    }

    #[test]
    fn side_to_move_alternates() {
        let mut state = GameState::new();
        assert_eq!(state.side_to_move, Color::White);
        let action = PgnParser::new().parse("e4").unwrap()[0].actions[0];
        state.process_move(action).unwrap();
        assert_eq!(state.side_to_move, Color::Black);
    }

    // ---- full famous games, final boards verified against python-chess ----

    #[test]
    fn fools_mate() {
        assert_eq!(
            play_game("1. f3 e5 2. g4 Qh4#"),
            normalize(
                "rnb.kbnr
                 pppp.ppp
                 ........
                 ....p...
                 ......Pq
                 .....P..
                 PPPPP..P
                 RNBQKBNR"
            )
        );
    }

    #[test]
    fn scholars_mate() {
        assert_eq!(
            play_game("1. e4 e5 2. Bc4 Nc6 3. Qh5 Nf6 4. Qxf7#"),
            normalize(
                "r.bqkb.r
                 pppp.Qpp
                 ..n..n..
                 ....p...
                 ..B.P...
                 ........
                 PPPP.PPP
                 RNB.K.NR"
            )
        );
    }

    #[test]
    fn both_sides_castle_kingside() {
        assert_eq!(
            play_game("1. e4 e5 2. Nf3 Nc6 3. Bc4 Bc5 4. O-O Nf6 5. d3 O-O"),
            normalize(
                "r.bq.rk.
                 pppp.ppp
                 ..n..n..
                 ..b.p...
                 ..B.P...
                 ...P.N..
                 PPP..PPP
                 RNBQ.RK."
            )
        );
    }

    #[test]
    fn en_passant_capture() {
        assert_eq!(
            play_game("1. e4 Nf6 2. e5 d5 3. exd6"),
            normalize(
                "rnbqkb.r
                 ppp.pppp
                 ...P.n..
                 ........
                 ........
                 ........
                 PPPP.PPP
                 RNBQKBNR"
            )
        );
    }

    #[test]
    fn double_promotion_with_captures() {
        assert_eq!(
            play_game("1. a4 h5 2. a5 h4 3. a6 h3 4. axb7 hxg2 5. bxa8=Q gxh1=Q"),
            normalize(
                "Qnbqkbnr
                 p.ppppp.
                 ........
                 ........
                 ........
                 ........
                 .PPPPP.P
                 RNBQKBNq"
            )
        );
    }

    #[test]
    fn opera_game() {
        let pgn = "1. e4 e5 2. Nf3 d6 3. d4 Bg4 4. dxe5 Bxf3 5. Qxf3 dxe5 6. Bc4 Nf6 \
                   7. Qb3 Qe7 8. Nc3 c6 9. Bg5 b5 10. Nxb5 cxb5 11. Bxb5+ Nbd7 \
                   12. O-O-O Rd8 13. Rxd7 Rxd7 14. Rd1 Qe6 15. Bxd7+ Nxd7 \
                   16. Qb8+ Nxb8 17. Rd8# 1-0";
        assert_eq!(
            play_game(pgn),
            normalize(
                ".n.Rkb.r
                 p....ppp
                 ....q...
                 ....p.B.
                 ....P...
                 ........
                 PPP..PPP
                 ..K....."
            )
        );
    }

    #[test]
    fn immortal_game() {
        let pgn = "1. e4 e5 2. f4 exf4 3. Bc4 Qh4+ 4. Kf1 b5 5. Bxb5 Nf6 6. Nf3 Qh6 \
                   7. d3 Nh5 8. Nh4 Qg5 9. Nf5 c6 10. g4 Nf6 11. Rg1 cxb5 12. h4 Qg6 \
                   13. h5 Qg5 14. Qf3 Ng8 15. Bxf4 Qf6 16. Nc3 Bc5 17. Nd5 Qxb2 \
                   18. Bd6 Bxg1 19. e5 Qxa1+ 20. Ke2 Na6 21. Nxg7+ Kd8 22. Qf6+ Nxf6 \
                   23. Be7# 1-0";
        assert_eq!(
            play_game(pgn),
            normalize(
                "r.bk...r
                 p..pBpNp
                 n....n..
                 .p.NP..P
                 ......P.
                 ...P....
                 P.P.K...
                 q.....b."
            )
        );
    }

    #[test]
    fn evergreen_game() {
        let pgn = "1. e4 e5 2. Nf3 Nc6 3. Bc4 Bc5 4. b4 Bxb4 5. c3 Ba5 6. d4 exd4 \
                   7. O-O d3 8. Qb3 Qf6 9. e5 Qg6 10. Re1 Nge7 11. Ba3 b5 12. Qxb5 Rb8 \
                   13. Qa4 Bb6 14. Nbd2 Bb7 15. Ne4 Qf5 16. Bxd3 Qh5 17. Nf6+ gxf6 \
                   18. exf6 Rg8 19. Rad1 Qxf3 20. Rxe7+ Nxe7 21. Qxd7+ Kxd7 22. Bf5+ Ke8 \
                   23. Bd7+ Kf8 24. Bxe7# 1-0";
        assert_eq!(
            play_game(pgn),
            normalize(
                ".r...kr.
                 pbpBBp.p
                 .b...P..
                 ........
                 ........
                 ..P..q..
                 P....PPP
                 ...R..K."
            )
        );
    }

    #[test]
    fn reti_versus_tartakower() {
        let pgn = "1. e4 c6 2. d4 d5 3. Nc3 dxe4 4. Nxe4 Nf6 5. Qd3 e5 6. dxe5 Qa5+ \
                   7. Bd2 Qxe5 8. O-O-O Nxe4 9. Qd8+ Kxd8 10. Bg5+ Kc7 11. Bd8#";
        assert_eq!(
            play_game(pgn),
            normalize(
                "rnbB.b.r
                 ppk..ppp
                 ..p.....
                 ....q...
                 ....n...
                 ........
                 PPP..PPP
                 ..KR.BNR"
            )
        );
    }

    // ---- corner cases built from ascii, verified against python-chess ----

    const CASTLE_SETUP: &str = "....k...
                                ........
                                ........
                                ........
                                ........
                                ........
                                ........
                                R...K..R";

    #[test]
    fn white_castles_kingside() {
        assert_eq!(
            play_from(CASTLE_SETUP, "O-O"),
            normalize(
                "....k...
                 ........
                 ........
                 ........
                 ........
                 ........
                 ........
                 R....RK."
            )
        );
    }

    #[test]
    fn white_castles_queenside() {
        assert_eq!(
            play_from(CASTLE_SETUP, "O-O-O"),
            normalize(
                "....k...
                 ........
                 ........
                 ........
                 ........
                 ........
                 ........
                 ..KR...R"
            )
        );
    }

    #[test]
    fn under_promotion_to_knight() {
        let setup = "k.......
                     ....P...
                     ........
                     ........
                     ........
                     ........
                     ........
                     ....K...";
        assert_eq!(
            play_from(setup, "e8=N"),
            normalize(
                "k...N...
                 ........
                 ........
                 ........
                 ........
                 ........
                 ........
                 ....K..."
            )
        );
    }

    #[test]
    fn capture_promotion_to_queen() {
        let setup = "r...k...
                     .P......
                     ........
                     ........
                     ........
                     ........
                     ........
                     ....K...";
        assert_eq!(
            play_from(setup, "bxa8=Q"),
            normalize(
                "Q...k...
                 ........
                 ........
                 ........
                 ........
                 ........
                 ........
                 ....K..."
            )
        );
    }

    #[test]
    fn promotion_giving_check() {
        let setup = "....k...
                     ..P.....
                     ........
                     ........
                     ........
                     ........
                     ........
                     ....K...";
        assert_eq!(
            play_from(setup, "c8=Q+"),
            normalize(
                "..Q.k...
                 ........
                 ........
                 ........
                 ........
                 ........
                 ........
                 ....K..."
            )
        );
    }

    // ================================================================
    // Edge cases that stretch the rules. Every expected board here was
    // produced independently by python-chess, which also verified that each
    // position is legal and that each move is the only legal reading of its
    // SAN.
    // ================================================================

    /// Two knights (b1, f3) both reach d2, but f3 is pinned to the king by the
    /// rook on f8. SAN therefore carries no disambiguation hint at all, so the
    /// resolver must exclude the pinned knight on its own.
    #[test]
    fn pin_makes_san_unambiguous_for_knights() {
        let setup = ".....r.k
                     ........
                     ........
                     ........
                     ........
                     .....N..
                     ........
                     .N...K..";
        assert_eq!(
            play_from(setup, "Nd2"),
            normalize(
                ".....r.k
                 ........
                 ........
                 ........
                 ........
                 .....N..
                 ...N....
                 .....K.."
            )
        );
    }

    /// Same idea for rooks: d5 and e2 both reach d2, but e2 is pinned along the
    /// e-file, so SAN is a bare `Rd2`.
    #[test]
    fn pin_makes_san_unambiguous_for_rooks() {
        let setup = "....r..k
                     ........
                     ........
                     ...R....
                     ........
                     ........
                     ....R...
                     ....K...";
        assert_eq!(
            play_from(setup, "Rd2"),
            normalize(
                "....r..k
                 ........
                 ........
                 ........
                 ........
                 ........
                 ...RR...
                 ....K..."
            )
        );
    }

    /// Both rooks sit on the a-file, but a black pawn on a3 blocks the a1 rook,
    /// so only the a5 rook can reach a4 and SAN carries no hint. This exercises
    /// path blocking in the slider candidate search.
    #[test]
    fn blocked_slider_is_not_a_candidate() {
        let setup = ".......k
                     ........
                     ........
                     R.......
                     ........
                     p.......
                     ........
                     R....K..";
        assert_eq!(
            play_from(setup, "Ra4"),
            normalize(
                ".......k
                 ........
                 ........
                 ........
                 R.......
                 p.......
                 ........
                 R....K.."
            )
        );
    }

    /// Three queens (a1, a5, e1) all reach e5, so the a1 queen needs both a file
    /// and a rank hint: `Qa1e5`.
    #[test]
    fn queen_needs_full_square_disambiguation() {
        let setup = ".......k
                     ........
                     ........
                     Q.......
                     ........
                     ........
                     ........
                     Q...Q.K.";
        assert_eq!(
            play_from(setup, "Qa1e5+"),
            normalize(
                ".......k
                 ........
                 ........
                 Q...Q...
                 ........
                 ........
                 ........
                 ....Q.K."
            )
        );
    }

    /// Three knights (a1, a5, c1) all reach b3, so the a1 knight needs both a
    /// file and a rank hint: `Na1b3`.
    #[test]
    fn knight_needs_full_square_disambiguation() {
        let setup = ".......k
                     ........
                     ........
                     N.......
                     ........
                     ........
                     ........
                     N.N..K..";
        assert_eq!(
            play_from(setup, "Na1b3"),
            normalize(
                ".......k
                 ........
                 ........
                 N.......
                 ........
                 .N......
                 ........
                 ..N..K.."
            )
        );
    }

    /// Two rooks on the same file are told apart by rank: `R1a3`.
    #[test]
    fn rook_rank_disambiguation() {
        let setup = ".......k
                     ........
                     ........
                     R.......
                     ........
                     ........
                     ........
                     R....K..";
        assert_eq!(
            play_from(setup, "R1a3"),
            normalize(
                ".......k
                 ........
                 ........
                 R.......
                 ........
                 R.......
                 ........
                 .....K.."
            )
        );
    }

    /// Two knights on the same file are told apart by rank: `N2d3`.
    #[test]
    fn knight_rank_disambiguation() {
        let setup = ".......k
                     ........
                     ........
                     ........
                     .N......
                     ........
                     .N......
                     .....K..";
        assert_eq!(
            play_from(setup, "N2d3"),
            normalize(
                ".......k
                 ........
                 ........
                 ........
                 .N......
                 ...N....
                 ........
                 .....K.."
            )
        );
    }

    #[test]
    fn under_promotion_to_rook() {
        let setup = "k.......
                     ..P.....
                     ........
                     ........
                     ........
                     ........
                     ........
                     ....K...";
        assert_eq!(
            play_from(setup, "c8=R+"),
            normalize(
                "k.R.....
                 ........
                 ........
                 ........
                 ........
                 ........
                 ........
                 ....K..."
            )
        );
    }

    #[test]
    fn under_promotion_to_bishop() {
        let setup = "k.......
                     ..P.....
                     ........
                     ........
                     ........
                     ........
                     ........
                     ....K...";
        assert_eq!(
            play_from(setup, "c8=B"),
            normalize(
                "k.B.....
                 ........
                 ........
                 ........
                 ........
                 ........
                 ........
                 ....K..."
            )
        );
    }

    /// Promoting to a knight while two knights are already on the board.
    #[test]
    fn promotion_creates_a_third_knight() {
        let setup = "r......k
                     .P......
                     ........
                     ........
                     ........
                     ........
                     ........
                     N.N..K..";
        assert_eq!(
            play_from(setup, "bxa8=N"),
            normalize(
                "N......k
                 ........
                 ........
                 ........
                 ........
                 ........
                 ........
                 N.N..K.."
            )
        );
    }

    /// Either pawn could capture on c8 and promote, so the file hint decides.
    #[test]
    fn capture_promotion_disambiguated_by_file() {
        let setup = "..r....k
                     .P.P....
                     ........
                     ........
                     ........
                     ........
                     ........
                     ....K...";
        assert_eq!(
            play_from(setup, "bxc8=Q+"),
            normalize(
                "..Q....k
                 ...P....
                 ........
                 ........
                 ........
                 ........
                 ........
                 ....K..."
            )
        );
    }

    /// En passant that also gives check. The captured pawn sits beside the
    /// capturing pawn, not on the destination square.
    #[test]
    fn en_passant_giving_check() {
        let setup = "........
                     ..kp....
                     ........
                     ....P...
                     ........
                     ........
                     P.......
                     ....K...";
        assert_eq!(
            play_from(setup, "1. a3 d5 2. exd6+"),
            normalize(
                "........
                 ..k.....
                 ...P....
                 ........
                 ........
                 P.......
                 ........
                 ....K..."
            )
        );
    }

    /// Castling queenside is legal even though b1 is attacked: the king never
    /// crosses b1, only the rook does.
    #[test]
    fn queenside_castle_with_b1_attacked() {
        let setup = ".r....k.
                     ........
                     ........
                     ........
                     ........
                     ........
                     ........
                     R...K..R";
        assert_eq!(
            play_from(setup, "O-O-O"),
            normalize(
                ".r....k.
                 ........
                 ........
                 ........
                 ........
                 ........
                 ........
                 ..KR...R"
            )
        );
    }

    // ---- further famous games, chosen for the notation they exercise ----

    /// Kasparov versus Topalov, Wijk aan Zee 1999. Includes `Nbxd5` (a
    /// file-disambiguated knight capture) and a long king hunt.
    #[test]
    fn kasparov_topalov() {
        let pgn = "1. e4 d6 2. d4 Nf6 3. Nc3 g6 4. Be3 Bg7 5. Qd2 c6 6. f3 b5 7. Nge2 Nbd7 \
                   8. Bh6 Bxh6 9. Qxh6 Bb7 10. a3 e5 11. O-O-O Qe7 12. Kb1 a6 13. Nc1 O-O-O \
                   14. Nb3 exd4 15. Rxd4 c5 16. Rd1 Nb6 17. g3 Kb8 18. Na5 Ba8 19. Bh3 d5 \
                   20. Qf4+ Ka7 21. Rhe1 d4 22. Nd5 Nbxd5 23. exd5 Qd6 24. Rxd4 cxd4 \
                   25. Re7+ Kb6 26. Qxd4+ Kxa5 27. b4+ Ka4 28. Qc3 Qxd5 29. Ra7 Bb7 \
                   30. Rxb7 Qc4 31. Qxf6 Kxa3 32. Qxa6+ Kxb4 33. c3+ Kxc3 34. Qa1+ Kd2 \
                   35. Qb2+ Kd1 36. Bf1 Rd2 37. Rd7 Rxd7 38. Bxc4 bxc4 39. Qxh8 Rd3 \
                   40. Qa8 c3 41. Qa4+ Ke1 42. f4 f5 43. Kc1 Rd2 44. Qa7 1-0";
        assert_eq!(
            play_game(pgn),
            normalize(
                "........
                 Q......p
                 ......p.
                 .....p..
                 .....P..
                 ..p...P.
                 ...r...P
                 ..K.k..."
            )
        );
    }

    /// Byrne versus Fischer, New York 1956 (the Game of the Century). Includes
    /// `Rfe8+` and a cascade of knight checks.
    #[test]
    fn byrne_fischer() {
        let pgn = "1. Nf3 Nf6 2. c4 g6 3. Nc3 Bg7 4. d4 O-O 5. Bf4 d5 6. Qb3 dxc4 7. Qxc4 c6 \
                   8. e4 Nbd7 9. Rd1 Nb6 10. Qc5 Bg4 11. Bg5 Na4 12. Qa3 Nxc3 13. bxc3 Nxe4 \
                   14. Bxe7 Qb6 15. Bc4 Nxc3 16. Bc5 Rfe8+ 17. Kf1 Be6 18. Bxb6 Bxc4+ \
                   19. Kg1 Ne2+ 20. Kf1 Nxd4+ 21. Kg1 Ne2+ 22. Kf1 Nc3+ 23. Kg1 axb6 \
                   24. Qb4 Ra4 25. Qxb6 Nxd1 26. h3 Rxa2 27. Kh2 Nxf2 28. Re1 Rxe1 \
                   29. Qd8+ Bf8 30. Nxe1 Bd5 31. Nf3 Ne4 32. Qb8 b5 33. h4 h5 34. Ne5 Kg7 \
                   35. Kg1 Bc5+ 36. Kf1 Ng3+ 37. Ke1 Bb4+ 38. Kd1 Bb3+ 39. Kc1 Ne2+ \
                   40. Kb1 Nc3+ 41. Kc1 Rc2# 0-1";
        assert_eq!(
            play_game(pgn),
            normalize(
                ".Q......
                 .....pk.
                 ..p...p.
                 .p..N..p
                 .b.....P
                 .bn.....
                 ..r...P.
                 ..K....."
            )
        );
    }

    /// Lasker versus Thomas, London 1912. Includes `Neg4+` and a black king
    /// marched all the way to g1.
    #[test]
    fn lasker_thomas() {
        let pgn = "1. d4 e6 2. Nf3 f5 3. Nc3 Nf6 4. Bg5 Be7 5. Bxf6 Bxf6 6. e4 fxe4 7. Nxe4 b6 \
                   8. Ne5 O-O 9. Bd3 Bb7 10. Qh5 Qe7 11. Qxh7+ Kxh7 12. Nxf6+ Kh6 \
                   13. Neg4+ Kg5 14. h4+ Kf4 15. g3+ Kf3 16. Be2+ Kg2 17. Rh2+ Kg1 18. Kd2# 1-0";
        assert_eq!(
            play_game(pgn),
            normalize(
                "rn...r..
                 pbppq.p.
                 .p..pN..
                 ........
                 ...P..NP
                 ......P.
                 PPPKBP.R
                 R.....k."
            )
        );
    }

    /// Steinitz versus von Bardeleben, Hastings 1895. Includes `Rac1` and
    /// `Rhc8` file disambiguation.
    #[test]
    fn steinitz_von_bardeleben() {
        let pgn = "1. e4 e5 2. Nf3 Nc6 3. Bc4 Bc5 4. c3 Nf6 5. d4 exd4 6. cxd4 Bb4+ 7. Nc3 d5 \
                   8. exd5 Nxd5 9. O-O Be6 10. Bg5 Be7 11. Bxd5 Bxd5 12. Nxd5 Qxd5 13. Bxe7 Nxe7 \
                   14. Re1 f6 15. Qe2 Qd7 16. Rac1 c6 17. d5 cxd5 18. Nd4 Kf7 19. Ne6 Rhc8 \
                   20. Qg4 g6 21. Ng5+ Ke8 22. Rxe7+ Kf8 23. Rf7+ Kg8 24. Rg7+ Kh8 25. Rxh7+ 1-0";
        assert_eq!(
            play_game(pgn),
            normalize(
                "r.r....k
                 pp.q...R
                 .....pp.
                 ...p..N.
                 ......Q.
                 ........
                 PP...PPP
                 ..R...K."
            )
        );
    }

    /// Botvinnik versus Capablanca, AVRO 1938. Includes an en passant capture
    /// (`exf6`) in the middle of a real game, plus `Rae1` and `Rfe8`.
    #[test]
    fn botvinnik_capablanca() {
        let pgn = "1. d4 Nf6 2. c4 e6 3. Nc3 Bb4 4. e3 d5 5. a3 Bxc3+ 6. bxc3 c5 7. cxd5 exd5 \
                   8. Bd3 O-O 9. Ne2 b6 10. O-O Ba6 11. Bxa6 Nxa6 12. Bb2 Qd7 13. a4 Rfe8 \
                   14. Qd3 c4 15. Qc2 Nb8 16. Rae1 Nc6 17. Ng3 Na5 18. f3 Nb3 19. e4 Qxa4 \
                   20. e5 Nd7 21. Qf2 g6 22. f4 f5 23. exf6 Nxf6 24. f5 Rxe1 25. Rxe1 Re8 \
                   26. Re6 Rxe6 27. fxe6 Kg7 28. Qf4 Qe8 29. Qe5 Qe7 30. Ba3 Qxa3 31. Nh5+ gxh5 \
                   32. Qg5+ Kf8 33. Qxf6+ Kg8 34. e7 Qc1+ 35. Kf2 Qc2+ 36. Kg3 Qd3+ 37. Kh4 Qe4+ \
                   38. Kxh5 Qe2+ 39. Kh4 Qe4+ 40. g4 Qe1+ 41. Kh5 1-0";
        assert_eq!(
            play_game(pgn),
            normalize(
                "......k.
                 p...P..p
                 .p...Q..
                 ...p...K
                 ..pP..P.
                 .nP.....
                 .......P
                 ....q..."
            )
        );
    }

    /// Rotlewi versus Rubinstein, Lodz 1907. Includes `Rac8` and a famous
    /// finish where black is a queen down.
    #[test]
    fn rotlewi_rubinstein() {
        let pgn = "1. d4 d5 2. Nf3 e6 3. e3 c5 4. c4 Nc6 5. Nc3 Nf6 6. dxc5 Bxc5 7. a3 a6 \
                   8. b4 Bd6 9. Bb2 O-O 10. Qd2 Qe7 11. Bd3 dxc4 12. Bxc4 b5 13. Bd3 Rd8 \
                   14. Qe2 Bb7 15. O-O Ne5 16. Nxe5 Bxe5 17. f4 Bc7 18. e4 Rac8 19. e5 Bb6+ \
                   20. Kh1 Ng4 21. Be4 Qh4 22. g3 Rxc3 23. gxh4 Rd2 24. Qxd2 Bxe4+ 25. Qg2 Rh3 0-1";
        assert_eq!(
            play_game(pgn),
            normalize(
                "......k.
                 .....ppp
                 pb..p...
                 .p..P...
                 .P..bPnP
                 P......r
                 .B....QP
                 R....R.K"
            )
        );
    }
}
