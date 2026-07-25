//! A parsed representation of a single chess move in Standard Algebraic
//! Notation (SAN). This module has no dependencies on other project modules.
//!
//! The representation is intentionally close to what SAN encodes rather than to
//! a fully resolved board change: SAN does not always spell out the origin
//! square (for example `Nf3`), so the source is stored as optional file and
//! rank hints. Resolving those hints into an exact origin square is the job of
//! the game state, not of this type.

use std::fmt;
use tudi::Coordinate;
use tudi::Positioned;

/// The kind of a chess piece, independent of colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PieceKind {
    Pawn,
    Knight,
    Bishop,
    Rook,
    Queen,
    King,
}

impl PieceKind {
    /// The uppercase SAN letter for this piece. A pawn has no letter in SAN, so
    /// this returns `None` for a pawn.
    pub fn san_letter(self) -> Option<char> {
        match self {
            PieceKind::Pawn => None,
            PieceKind::Knight => Some('N'),
            PieceKind::Bishop => Some('B'),
            PieceKind::Rook => Some('R'),
            PieceKind::Queen => Some('Q'),
            PieceKind::King => Some('K'),
        }
    }
}

/// The file index of the origin square d4: files a..=h are 0..=7, so d = 3.
const ORIGIN_FILE_INDEX: i32 = 3;
/// The rank index of the origin square d4: ranks 1..=8 are 0..=7, so rank 4 is
/// index 3.
const ORIGIN_RANK_INDEX: i32 = 3;

/// A square on the board.
///
/// Internally a square is a tudi [`Coordinate`] on an origin-centered grid where
/// **d4 is the origin `(0, 0)`**. Files a..=h map to x = -3..=4 and ranks 1..=8
/// map to y = -3..=4, which is exactly the extent of a tudi `Grid::new(8, 8)`.
/// Because of this a `Square` can be handed directly to any tudi API that
/// expects a [`Positioned`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Square {
    coordinate: Coordinate,
}

impl Square {
    /// Builds a square from a zero-based file (0..=7 for a..=h) and a zero-based
    /// rank (0..=7 for 1..=8).
    pub fn new(file: u8, rank: u8) -> Self {
        let x = i32::from(file) - ORIGIN_FILE_INDEX;
        let y = i32::from(rank) - ORIGIN_RANK_INDEX;
        Square {
            coordinate: Coordinate { x, y },
        }
    }

    /// The zero-based file (0..=7 for a..=h).
    pub fn file(self) -> u8 {
        (self.coordinate.x + ORIGIN_FILE_INDEX) as u8
    }

    /// The zero-based rank (0..=7 for 1..=8).
    pub fn rank(self) -> u8 {
        (self.coordinate.y + ORIGIN_RANK_INDEX) as u8
    }
}

impl Positioned for Square {
    fn position(&self) -> &Coordinate {
        &self.coordinate
    }
}

impl fmt::Display for Square {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let file_letter = (b'a' + self.file()) as char;
        let rank_digit = (b'1' + self.rank()) as char;
        write!(formatter, "{file_letter}{rank_digit}")
    }
}

/// Whether a move gives check, checkmate, or neither. This is cosmetic (it does
/// not change the board) but is kept so the original SAN can be shown faithfully
/// in the move list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CheckIndicator {
    None,
    Check,
    Checkmate,
}

impl CheckIndicator {
    fn suffix(self) -> &'static str {
        match self {
            CheckIndicator::None => "",
            CheckIndicator::Check => "+",
            CheckIndicator::Checkmate => "#",
        }
    }
}

/// The three structurally different kinds of move.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ActionKind {
    /// Any ordinary move, including captures, en passant, and pawn promotions.
    Normal {
        piece: PieceKind,
        /// The origin file, if SAN specified it (disambiguation or a pawn
        /// capture such as `exd5`).
        source_file: Option<u8>,
        /// The origin rank, if SAN specified it (disambiguation).
        source_rank: Option<u8>,
        destination: Square,
        is_capture: bool,
        /// The piece a pawn promotes to, if this is a promotion.
        promotion: Option<PieceKind>,
    },
    CastleKingside,
    CastleQueenside,
}

/// A single fully parsed SAN move.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Action {
    pub kind: ActionKind,
    pub check: CheckIndicator,
}

impl Action {
    pub fn new(kind: ActionKind, check: CheckIndicator) -> Self {
        Action { kind, check }
    }
}

impl fmt::Display for Action {
    /// Reconstructs the SAN text for the move so it can be shown in the move
    /// list.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            ActionKind::CastleKingside => write!(formatter, "O-O{}", self.check.suffix()),
            ActionKind::CastleQueenside => write!(formatter, "O-O-O{}", self.check.suffix()),
            ActionKind::Normal {
                piece,
                source_file,
                source_rank,
                destination,
                is_capture,
                promotion,
            } => {
                if let Some(letter) = piece.san_letter() {
                    write!(formatter, "{letter}")?;
                }
                if let Some(file) = source_file {
                    write!(formatter, "{}", (b'a' + file) as char)?;
                }
                if let Some(rank) = source_rank {
                    write!(formatter, "{}", (b'1' + rank) as char)?;
                }
                if is_capture {
                    write!(formatter, "x")?;
                }
                write!(formatter, "{destination}")?;
                if let Some(promoted) = promotion {
                    if let Some(letter) = promoted.san_letter() {
                        write!(formatter, "={letter}")?;
                    }
                }
                write!(formatter, "{}", self.check.suffix())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn square_displays_as_algebraic() {
        assert_eq!(Square::new(0, 0).to_string(), "a1");
        assert_eq!(Square::new(4, 3).to_string(), "e4");
        assert_eq!(Square::new(7, 7).to_string(), "h8");
    }

    #[test]
    fn d4_is_the_origin_coordinate() {
        assert_eq!(Square::new(3, 3).position(), &Coordinate { x: 0, y: 0 }); // d4
        assert_eq!(Square::new(0, 0).position(), &Coordinate { x: -3, y: -3 }); // a1
        assert_eq!(Square::new(7, 7).position(), &Coordinate { x: 4, y: 4 }); // h8
    }

    #[test]
    fn file_and_rank_round_trip() {
        for file in 0..8u8 {
            for rank in 0..8u8 {
                let square = Square::new(file, rank);
                assert_eq!(square.file(), file);
                assert_eq!(square.rank(), rank);
            }
        }
    }

    #[test]
    fn pawn_push_reconstructs() {
        let action = Action::new(
            ActionKind::Normal {
                piece: PieceKind::Pawn,
                source_file: None,
                source_rank: None,
                destination: Square::new(4, 3),
                is_capture: false,
                promotion: None,
            },
            CheckIndicator::None,
        );
        assert_eq!(action.to_string(), "e4");
    }

    #[test]
    fn pawn_capture_reconstructs() {
        let action = Action::new(
            ActionKind::Normal {
                piece: PieceKind::Pawn,
                source_file: Some(4),
                source_rank: None,
                destination: Square::new(3, 4),
                is_capture: true,
                promotion: None,
            },
            CheckIndicator::None,
        );
        assert_eq!(action.to_string(), "exd5");
    }

    #[test]
    fn disambiguated_knight_reconstructs() {
        let action = Action::new(
            ActionKind::Normal {
                piece: PieceKind::Knight,
                source_file: Some(1),
                source_rank: None,
                destination: Square::new(3, 1),
                is_capture: false,
                promotion: None,
            },
            CheckIndicator::None,
        );
        assert_eq!(action.to_string(), "Nbd2");
    }

    #[test]
    fn promotion_with_check_reconstructs() {
        let action = Action::new(
            ActionKind::Normal {
                piece: PieceKind::Pawn,
                source_file: None,
                source_rank: None,
                destination: Square::new(4, 7),
                is_capture: false,
                promotion: Some(PieceKind::Queen),
            },
            CheckIndicator::Check,
        );
        assert_eq!(action.to_string(), "e8=Q+");
    }

    #[test]
    fn castles_reconstruct() {
        let kingside = Action::new(ActionKind::CastleKingside, CheckIndicator::None);
        let queenside = Action::new(ActionKind::CastleQueenside, CheckIndicator::Checkmate);
        assert_eq!(kingside.to_string(), "O-O");
        assert_eq!(queenside.to_string(), "O-O-O#");
    }
}
