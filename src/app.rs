//! The terminal application: a PGN viewer built on ratatui.
//!
//! The app has two modes. A PGN file may hold many games, so [`Mode::Menu`]
//! lists them and lets one be picked; [`Mode::Board`] then shows that game with
//! the board in the centre, the move list on the right, and buttons along the
//! bottom. A file holding a single game opens straight in board mode. `Esc` (or
//! the "all games" button) goes back to the menu and `q` quits.
//!
//! This is the application layer: it holds the parsed games plus a cursor into
//! the one on screen, reads the PGN file from the OS data directory, and reports
//! problems with miette. The board itself is produced by the `game` module.

use crate::action::Action;
use crate::action::ActionKind;
use crate::game::GameState;
use crate::parser::PgnParser;

use std::ffi::OsString;
use std::io;
use std::io::Stdout;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use miette::miette;
use miette::Context;
use miette::IntoDiagnostic;
use miette::Result;

use ratatui::backend::CrosstermBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Alignment;
use ratatui::layout::Constraint;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Block;
use ratatui::widgets::Borders;
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use ratatui::Terminal;

use ratatui::crossterm::event;
use ratatui::crossterm::event::DisableMouseCapture;
use ratatui::crossterm::event::EnableMouseCapture;
use ratatui::crossterm::event::Event;
use ratatui::crossterm::event::KeyCode;
use ratatui::crossterm::event::KeyEventKind;
use ratatui::crossterm::event::MouseButton;
use ratatui::crossterm::event::MouseEventKind;
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::disable_raw_mode;
use ratatui::crossterm::terminal::enable_raw_mode;
use ratatui::crossterm::terminal::EnterAlternateScreen;
use ratatui::crossterm::terminal::LeaveAlternateScreen;
// ===========================================================================
// CONFIGURATION — everything meant to be tweaked lives in this block.
// ===========================================================================

/// Colours and glyphs. Change these to restyle the whole app.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    /// Background of a light square.
    pub light_square: Color,
    /// Background of a dark square.
    pub dark_square: Color,
    /// A light square that was part of the last move.
    pub light_square_last_move: Color,
    /// A dark square that was part of the last move.
    pub dark_square_last_move: Color,
    /// Foreground colour used to draw white pieces.
    pub white_piece: Color,
    /// Foreground colour used to draw black pieces.
    pub black_piece: Color,
    /// Colour of the file/rank coordinate labels around the board.
    pub coordinate_label: Color,

    /// Panel background (board panel, move list, buttons).
    pub panel_background: Color,
    /// Panel border colour.
    pub panel_border: Color,

    /// The name of the game on show, above the board.
    pub header_text: Color,

    /// Move-number prefix colour in the move list.
    pub move_number: Color,
    /// Normal (non-current) move text colour.
    pub move_text: Color,
    /// Background of the current move in the list.
    pub current_move_background: Color,
    /// Foreground of the current move in the list.
    pub current_move_foreground: Color,

    /// Game-number prefix colour in the menu.
    pub menu_number: Color,
    /// Normal (unselected) game name in the menu.
    pub menu_text: Color,
    /// Background of the selected game in the menu.
    pub menu_selected_background: Color,
    /// Foreground of the selected game in the menu.
    pub menu_selected_foreground: Color,

    /// Colour of a problem reported in the menu, such as a game that cannot be
    /// replayed.
    pub error_text: Color,

    /// Button label colour.
    pub button_foreground: Color,
    /// Button background colour.
    pub button_background: Color,
}

/// The default theme. Edit these values (or build your own `Theme`) to restyle.
pub const DEFAULT_THEME: Theme = Theme {
    // Board squares (opaque): two neutral greys, so the board stays solid and
    // readable over a transparent UI.
    light_square: Color::Rgb(0x50, 0x50, 0x50),
    dark_square: Color::Rgb(0x32, 0x32, 0x32),
    // Last-move squares: a muted amber tint (opaque).
    light_square_last_move: Color::Rgb(0x8F, 0x7D, 0x4F),
    dark_square_last_move: Color::Rgb(0x6F, 0x5F, 0x3A),
    // Piece colours for the block renderer; the image renderer uses the
    // artwork's own colours.
    white_piece: Color::Rgb(0xEC, 0xEC, 0xEC),
    black_piece: Color::Rgb(0x18, 0x18, 0x18),
    coordinate_label: Color::Rgb(0x80, 0x80, 0x80),

    // Panels are transparent: `Color::Reset` lets the terminal's own background
    // (and any transparency) show through. Set these to an Rgb colour to make
    // the UI opaque instead.
    panel_background: Color::Reset,
    panel_border: Color::Rgb(0x55, 0x55, 0x55),

    header_text: Color::Rgb(0xCC, 0xCC, 0xCC),

    move_number: Color::Rgb(0x70, 0x70, 0x70),
    move_text: Color::Rgb(0xCC, 0xCC, 0xCC),
    current_move_background: Color::Rgb(0x4A, 0x4A, 0x4A),
    current_move_foreground: Color::Rgb(0xFF, 0xFF, 0xFF),

    menu_number: Color::Rgb(0x70, 0x70, 0x70),
    menu_text: Color::Rgb(0xCC, 0xCC, 0xCC),
    menu_selected_background: Color::Rgb(0x4A, 0x4A, 0x4A),
    menu_selected_foreground: Color::Rgb(0xFF, 0xFF, 0xFF),

    error_text: Color::Rgb(0xC8, 0x76, 0x76),

    button_foreground: Color::Rgb(0xCC, 0xCC, 0xCC),
    button_background: Color::Reset,
};

/// Width, in columns, of the move-list panel on the right.
pub const MOVES_PANEL_WIDTH: u16 = 26;
/// Height, in rows, of the button bar at the bottom.
pub const BOTTOM_BAR_HEIGHT: u16 = 3;
/// Height, in rows, of the game-name header above the board.
pub const HEADER_HEIGHT: u16 = 1;
/// Minimum width, in columns, reserved for the board panel.
pub const BOARD_MIN_WIDTH: u16 = 24;
/// Width each move is padded to in the list, so the columns line up.
pub const MOVE_CELL_WIDTH: usize = 7;

/// How tall a terminal cell is relative to its width (font height ÷ font
/// width). Almost every terminal font is about twice as tall as it is wide, so
/// a board square needs roughly twice as many columns as rows to look square.
///
/// This is only the fallback: with the `images` feature the real font size is
/// queried from the terminal and used instead.
pub const DEFAULT_CELL_ASPECT: f64 = 2.0;

// ===========================================================================
// APPLICATION
// ===========================================================================

/// Which screen the app is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    /// The list of games in the file.
    Menu,
    /// One game's board and moves.
    Board,
}

/// Everything needed to draw a game once it has been opened.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Replay {
    /// The SAN text of each half-move, for the move list.
    san_moves: Vec<String>,
    /// The board (as ascii) after each ply. `positions[0]` is the start;
    /// `positions[ply]` is the position after `ply` half-moves.
    positions: Vec<String>,
}

/// One game from the file, and its replay once it has been opened.
#[derive(Debug, Clone, PartialEq, Eq)]
struct GameEntry {
    /// The name to show. Taken from the file's tag pairs, or `Game N` when the
    /// file does not name the game.
    name: String,
    actions: Vec<Action>,
    /// Built the first time the game is opened. Replaying every game up front
    /// would make a large file slow to start and would let one unplayable game
    /// stop the whole app from opening.
    replay: Option<Replay>,
}

/// The running application: the games from the file and a cursor into the one
/// on screen.
#[derive(Debug)]
pub struct App {
    games: Vec<GameEntry>,
    mode: Mode,
    /// The menu cursor.
    selected: usize,
    /// The game shown in board mode.
    current_game: usize,
    /// How many half-moves of the current game are applied (`0..=actions.len()`).
    ply: usize,
    /// A problem worth showing rather than crashing over, such as a game whose
    /// moves cannot be replayed.
    status: Option<String>,
    theme: Theme,
    should_quit: bool,
    /// The most recent drawing area, used to hit-test mouse clicks.
    viewport: Rect,
    /// Real-graphics backend, present only with the `images` feature and when a
    /// terminal graphics protocol was detected.
    #[cfg(feature = "images")]
    image_backend: Option<crate::image_backend::ImageBackend>,
}

/// Replays a game, producing the position after every half-move.
fn build_replay(actions: &[Action]) -> Result<Replay, String> {
    let mut state = GameState::new();
    let mut positions = Vec::with_capacity(actions.len() + 1);
    positions.push(state.to_ascii());
    for action in actions {
        state
            .process_move(*action)
            .map_err(|error| format!("could not apply move {action}: {error}"))?;
        positions.push(state.to_ascii());
    }
    Ok(Replay {
        san_moves: actions.iter().map(ToString::to_string).collect(),
        positions,
    })
}

impl App {
    /// Builds an app from PGN text.
    ///
    /// Games are not replayed here, only parsed; a game's positions are built
    /// when it is opened. A file with exactly one game opens straight onto the
    /// board, otherwise the menu is shown.
    ///
    /// # Errors
    ///
    /// Returns an error if the text is not valid PGN, or if it holds no games.
    pub fn from_pgn(pgn: &str) -> Result<Self> {
        let parsed = PgnParser::new()
            .parse(pgn)
            .map_err(|error| miette!("invalid PGN: {error}"))?;
        if parsed.is_empty() {
            return Err(miette!("the PGN holds no games"));
        }

        let games: Vec<GameEntry> = parsed
            .into_iter()
            .enumerate()
            .map(|(index, game)| GameEntry {
                // The parser reports only what the file says; naming the
                // nameless is a presentation choice, and it follows file order
                // so `Game 3` is always the third game.
                name: game.name.unwrap_or_else(|| format!("Game {}", index + 1)),
                actions: game.actions,
                replay: None,
            })
            .collect();

        let single_game = games.len() == 1;
        let mut app = App {
            games,
            mode: Mode::Menu,
            selected: 0,
            current_game: 0,
            ply: 0,
            status: None,
            theme: DEFAULT_THEME,
            should_quit: false,
            viewport: Rect::default(),
            #[cfg(feature = "images")]
            image_backend: None,
        };
        if single_game {
            // A menu of one is just an extra key press; the "all games" button
            // still leads back to it.
            app.open_game(0);
        }
        Ok(app)
    }

    /// Builds an app with no games, showing `message` in the menu. Used when no
    /// file was given on the command line and no default file could be loaded,
    /// so the program lands in the menu with an explanation rather than exiting.
    fn empty(message: impl Into<String>) -> Self {
        App {
            games: Vec::new(),
            mode: Mode::Menu,
            selected: 0,
            current_game: 0,
            ply: 0,
            status: Some(message.into()),
            theme: DEFAULT_THEME,
            should_quit: false,
            viewport: Rect::default(),
            #[cfg(feature = "images")]
            image_backend: None,
        }
    }

    /// Attempts to enable real-image rendering. If the terminal supports a
    /// graphics protocol, pieces are drawn as images; otherwise the block
    /// renderer is left in place.
    #[cfg(feature = "images")]
    pub fn enable_images(&mut self) {
        if let Ok(backend) = crate::image_backend::ImageBackend::new() {
            self.image_backend = Some(backend);
        }
    }

    // -- games -------------------------------------------------------------

    /// Opens a game on the board, building its replay the first time. A game
    /// that cannot be replayed leaves the menu up and reports why.
    fn open_game(&mut self, index: usize) {
        let Some(entry) = self.games.get_mut(index) else {
            return;
        };
        if entry.replay.is_none() {
            match build_replay(&entry.actions) {
                Ok(replay) => entry.replay = Some(replay),
                Err(message) => {
                    let name = entry.name.clone();
                    self.status = Some(format!("{name}: {message}"));
                    return;
                }
            }
        }
        self.current_game = index;
        self.ply = 0;
        self.status = None;
        self.mode = Mode::Board;
    }

    /// Returns to the menu, with the game that was on screen selected.
    fn show_menu(&mut self) {
        self.selected = self.current_game;
        self.mode = Mode::Menu;
    }

    fn current_replay(&self) -> Option<&Replay> {
        self.games
            .get(self.current_game)
            .and_then(|entry| entry.replay.as_ref())
    }

    fn current_actions(&self) -> &[Action] {
        self.games
            .get(self.current_game)
            .map_or(&[], |entry| entry.actions.as_slice())
    }

    // -- navigation --------------------------------------------------------

    fn forward(&mut self) {
        if self.ply < self.current_actions().len() {
            self.ply += 1;
        }
    }

    fn backward(&mut self) {
        self.ply = self.ply.saturating_sub(1);
    }

    fn reset(&mut self) {
        self.ply = 0;
    }

    fn go_to_end(&mut self) {
        self.ply = self.current_actions().len();
    }

    fn select_previous(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    fn select_next(&mut self) {
        if self.selected + 1 < self.games.len() {
            self.selected += 1;
        }
    }

    /// The first menu row on screen. Derived from the selection so the cursor is
    /// always visible, and shared by drawing and mouse hit-testing.
    fn menu_scroll(&self, visible: u16) -> u16 {
        let selected = u16::try_from(self.selected).unwrap_or(u16::MAX);
        if visible > 0 && selected >= visible {
            selected - visible + 1
        } else {
            0
        }
    }

    /// The squares of the most recent move, to highlight on the board.
    fn last_move_highlights(&self) -> Vec<(u8, u8)> {
        if self.ply == 0 {
            return Vec::new();
        }
        let actions = self.current_actions();
        let Some(action) = actions.get(self.ply - 1) else {
            return Vec::new();
        };
        // `ply` counts half-moves and is at least 1 here, so an odd count means
        // the move just played was white's.
        let white_moved = self.ply % 2 == 1;
        let back_rank: u8 = if white_moved { 0 } else { 7 };
        match action.kind {
            ActionKind::Normal { destination, .. } => {
                vec![(destination.file(), destination.rank())]
            }
            ActionKind::CastleKingside => vec![(6, back_rank), (5, back_rank)],
            ActionKind::CastleQueenside => vec![(2, back_rank), (3, back_rank)],
        }
    }

    // -- input -------------------------------------------------------------

    fn handle_key(&mut self, code: KeyCode) {
        match self.mode {
            Mode::Menu => self.handle_menu_key(code),
            Mode::Board => self.handle_board_key(code),
        }
    }

    fn handle_menu_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Up => self.select_previous(),
            KeyCode::Down => self.select_next(),
            KeyCode::Home => self.selected = 0,
            KeyCode::End => self.selected = self.games.len().saturating_sub(1),
            KeyCode::Enter => self.open_game(self.selected),
            KeyCode::Char('q' | 'Q') | KeyCode::Esc => self.should_quit = true,
            _ => {}
        }
    }

    fn handle_board_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Right => self.forward(),
            KeyCode::Left => self.backward(),
            KeyCode::Char('r' | 'R') | KeyCode::Home => self.reset(),
            KeyCode::End => self.go_to_end(),
            KeyCode::Char('a' | 'A') | KeyCode::Esc => self.show_menu(),
            KeyCode::Char('q' | 'Q') => self.should_quit = true,
            _ => {}
        }
    }

    fn handle_mouse(&mut self, kind: MouseEventKind, column: u16, row: u16) {
        if kind != MouseEventKind::Down(MouseButton::Left) {
            return;
        }
        match self.mode {
            Mode::Menu => {
                let regions = menu_layout(self.viewport);
                let inner = panel_block(self.theme).inner(regions.list);
                if hit(inner, column, row) {
                    let offset = row - inner.y;
                    let first = self.menu_scroll(inner.height) as usize;
                    let index = first + offset as usize;
                    if index < self.games.len() {
                        self.open_game(index);
                    }
                }
            }
            Mode::Board => {
                let regions = board_layout(self.viewport);
                if hit(regions.buttons[0], column, row) {
                    self.show_menu();
                } else if hit(regions.buttons[1], column, row) {
                    self.backward();
                } else if hit(regions.buttons[2], column, row) {
                    self.reset();
                } else if hit(regions.buttons[3], column, row) {
                    self.forward();
                }
            }
        }
    }

    // -- rendering ---------------------------------------------------------

    fn render(&mut self, frame: &mut Frame<'_>) {
        let area = frame.area();
        self.viewport = area;
        frame
            .buffer_mut()
            .set_style(area, Style::default().bg(self.theme.panel_background));

        match self.mode {
            Mode::Menu => self.render_menu_mode(frame, area),
            Mode::Board => self.render_board_mode(frame, area),
        }
    }

    fn render_menu_mode(&self, frame: &mut Frame<'_>, area: Rect) {
        let regions = menu_layout(area);
        self.render_menu(frame, regions.list);

        let hint = if self.games.len() == 1 {
            "Enter: open    q: quit"
        } else {
            "↑ ↓: select    Enter: open    q: quit"
        };
        let (text, colour) = match &self.status {
            Some(status) => (status.as_str(), self.theme.error_text),
            None => (hint, self.theme.button_foreground),
        };
        render_bottom_text(frame, regions.bottom, text, colour, self.theme);
    }

    fn render_menu(&self, frame: &mut Frame<'_>, area: Rect) {
        let inner = self.render_panel(frame, area);
        if inner.width == 0 || inner.height == 0 {
            return;
        }

        // Every list row is a game and nothing else: the row a click lands on is
        // the game it opens. Anything else here (a message, a heading) would
        // shift the rows out from under the mouse. Messages go in the bar below.
        let width = inner.width as usize;
        let mut lines: Vec<Line<'_>> = Vec::with_capacity(self.games.len());
        for (index, game) in self.games.iter().enumerate() {
            let moves = game.actions.len();
            let text = format!(" {:>3}. {}  ({moves} moves)", index + 1, game.name);
            let style = if index == self.selected {
                Style::default()
                    .bg(self.theme.menu_selected_background)
                    .fg(self.theme.menu_selected_foreground)
            } else {
                Style::default().fg(self.theme.menu_text)
            };
            // Pad to the full width so the selected row reads as a bar.
            lines.push(Line::from(Span::styled(fit(&text, width), style)));
        }

        let scroll = self.menu_scroll(inner.height);
        frame.render_widget(Paragraph::new(lines).scroll((scroll, 0)), inner);
    }

    fn render_board_mode(&self, frame: &mut Frame<'_>, area: Rect) {
        let regions = board_layout(area);
        self.render_header(frame, regions.header);
        self.render_board_panel(frame, regions.board);
        self.render_moves_panel(frame, regions.moves);

        let labels = [
            "◀ All games (esc)",
            "◀ Back (←)",
            "Reset (r)",
            "Forward (→) ▶",
        ];
        for (rect, label) in regions.buttons.iter().zip(labels) {
            render_button(frame, *rect, label, self.theme);
        }
    }

    /// The name of the game on show. Nothing else on screen says which game the
    /// board belongs to.
    fn render_header(&self, frame: &mut Frame<'_>, area: Rect) {
        let Some(entry) = self.games.get(self.current_game) else {
            return;
        };
        let text = fit(&format!(" {}", entry.name), area.width as usize);
        frame.render_widget(
            Paragraph::new(text).style(Style::default().fg(self.theme.header_text)),
            area,
        );
    }

    /// Draws a panel's border block into `area` and returns the inner area for
    /// its contents. Every panel (menu list, board, move list) shares this.
    fn render_panel(&self, frame: &mut Frame<'_>, area: Rect) -> Rect {
        let block = panel_block(self.theme);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        inner
    }

    fn render_board_panel(&self, frame: &mut Frame<'_>, area: Rect) {
        let inner = self.render_panel(frame, area);
        self.render_board(frame, inner);
    }
    /// How tall a terminal cell is relative to its width. Uses the font size
    /// reported by the terminal when real-image rendering is active, and falls
    /// back to [`DEFAULT_CELL_ASPECT`] otherwise.
    // `self` is only read when the `images` feature is enabled; clippy sees the
    // configuration where it is not.
    #[allow(clippy::unused_self)]
    fn cell_aspect(&self) -> f64 {
        #[cfg(feature = "images")]
        if let Some(backend) = &self.image_backend {
            let (font_width, font_height) = backend.font_size();
            if font_width > 0 && font_height > 0 {
                return f64::from(font_height) / f64::from(font_width);
            }
        }
        DEFAULT_CELL_ASPECT
    }

    /// Whether pieces are drawn as real graphics rather than block sprites.
    /// Always false without the `images` feature or when no terminal graphics
    /// protocol was detected.
    // `self` is only read when the `images` feature is enabled.
    #[allow(clippy::unused_self)]
    fn use_images(&self) -> bool {
        #[cfg(feature = "images")]
        {
            self.image_backend.is_some()
        }
        #[cfg(not(feature = "images"))]
        {
            false
        }
    }

    fn render_board(&self, frame: &mut Frame<'_>, area: Rect) {
        if let Some(geometry) = self.board_geometry(area) {
            self.draw_board(frame, &geometry);
        }
    }

    /// Works out where the grid sits and how big each square is, or `None` when
    /// the area is too small to draw a board.
    fn board_geometry(&self, area: Rect) -> Option<BoardGeometry> {
        // Layout: a rank-label column on the left, then the grid; below the grid
        // a blank spacer row, then the file-label row.
        let label_gutter = 2u16;
        let spacer_row = 1u16;
        let label_row = 1u16;
        let reserved_height = spacer_row + label_row;
        if area.width <= label_gutter || area.height <= reserved_height {
            return None;
        }

        // A board square should look square, which means its pixel width and
        // height must match: cell_width * font_width == cell_height * font_height.
        // Deriving the two sizes independently from the available space (as a
        // naive fit does) lets their ratio drift as the terminal is resized, so
        // instead the height is chosen first and the width follows from the
        // cell aspect. The square then keeps the same shape at every size.
        let aspect = self.cell_aspect();
        let available_width = area.width - label_gutter;
        let available_height = area.height - reserved_height;
        let width_limited_height = (f64::from(available_width) / 8.0 / aspect).floor() as u16;
        let cell_height = (available_height / 8).min(width_limited_height).max(1);
        let cell_width = ((f64::from(cell_height) * aspect).round() as u16)
            .min(available_width / 8)
            .max(1);
        let grid_width = cell_width * 8;
        let grid_height = cell_height * 8;

        // Cell sizes are floored at one, so below roughly ten cells the grid no
        // longer fits and centering it would underflow. Draw nothing instead.
        let block_height = grid_height + reserved_height;
        if grid_width + label_gutter > area.width || block_height > area.height {
            return None;
        }

        // Center the whole block (grid + spacer + labels) in the panel, so the
        // labels are not stranded far above the bottom border.
        let origin_x = area.x + label_gutter + (area.width - label_gutter - grid_width) / 2;
        let origin_y = area.y + (area.height - block_height) / 2;

        Some(BoardGeometry {
            origin_x,
            origin_y,
            cell_width,
            cell_height,
            grid_height,
        })
    }

    /// Draws the current position into the grid described by `geometry`.
    fn draw_board(&self, frame: &mut Frame<'_>, geometry: &BoardGeometry) {
        // Board mode is only entered once the replay is built and `ply` is kept
        // in range by the navigation, but a panic here would leave the terminal
        // in raw mode, so draw never assumes either.
        let Some(position) = self
            .current_replay()
            .and_then(|replay| replay.positions.get(self.ply))
        else {
            return;
        };
        // Decode the position into eight fixed rows of bytes once. Both this
        // function and the image pass below read squares through `cell_char`,
        // so a short or missing row yields an empty square rather than an
        // out-of-bounds index that would panic with the terminal in raw mode.
        let mut rows: [&[u8]; 8] = [&[]; 8];
        for (slot, line) in rows.iter_mut().zip(position.lines()) {
            *slot = line.as_bytes();
        }
        let highlights = self.last_move_highlights();

        let use_images = self.use_images();

        self.draw_squares_and_labels(frame, geometry, &rows, &highlights, use_images);

        // Second pass: draw pieces as real graphics via the image backend, on
        // top of the square backgrounds laid down in the first pass.
        #[cfg(feature = "images")]
        if use_images {
            let Some(backend) = &self.image_backend else {
                return;
            };
            for display_row in 0..8u16 {
                for file in 0..8u16 {
                    let character = cell_char(&rows, display_row, file);
                    if character != '.' {
                        backend.render(frame, geometry.cell(display_row, file), character);
                    }
                }
            }
        }
    }

    /// The background color for a square, given whether it is a light square and
    /// whether it was part of the most recent move.
    fn square_background(&self, is_light: bool, is_last_move: bool) -> Color {
        match (is_light, is_last_move) {
            (true, false) => self.theme.light_square,
            (false, false) => self.theme.dark_square,
            (true, true) => self.theme.light_square_last_move,
            (false, true) => self.theme.dark_square_last_move,
        }
    }

    /// The foreground color for a piece glyph. White pieces are uppercase in the
    /// board text and black pieces lowercase.
    fn piece_foreground(&self, character: char) -> Color {
        if character.is_ascii_uppercase() {
            self.theme.white_piece
        } else {
            self.theme.black_piece
        }
    }

    /// First pass into the text buffer: the squares with their pieces (unless
    /// images are used), then the rank and file coordinate labels. Each of the
    /// three is a self-contained loop.
    fn draw_squares_and_labels(
        &self,
        frame: &mut Frame<'_>,
        geometry: &BoardGeometry,
        rows: &[&[u8]; 8],
        highlights: &[(u8, u8)],
        use_images: bool,
    ) {
        let buffer = frame.buffer_mut();
        let label_style = Style::default().fg(self.theme.coordinate_label);

        // Squares, and the pieces standing on them.
        for display_row in 0..8u16 {
            let rank = 7 - display_row;
            for file in 0..8u16 {
                let cell = geometry.cell(display_row, file);
                let is_light = (file + rank) % 2 == 1;
                let is_last_move = highlights.contains(&(file as u8, rank as u8));
                let background = self.square_background(is_light, is_last_move);
                buffer.set_style(cell, Style::default().bg(background));

                let character = cell_char(rows, display_row, file);
                if character != '.' && !use_images {
                    let foreground = self.piece_foreground(character);
                    draw_piece(buffer, cell, character, foreground, background);
                }
            }
        }

        // Rank labels (8 down to 1) in the gutter left of the board.
        for display_row in 0..8u16 {
            let rank = 7 - display_row;
            let label = (b'1' + rank as u8) as char;
            let band_top = geometry.origin_y + display_row * geometry.cell_height;
            let x = geometry.origin_x.saturating_sub(2);
            let y = band_top + geometry.cell_height / 2;
            buffer.set_string(x, y, label.to_string(), label_style);
        }

        // File labels (a to h) in the row below the board.
        for file in 0..8u16 {
            let label = (b'a' + file as u8) as char;
            let x = geometry.origin_x + file * geometry.cell_width + geometry.cell_width / 2;
            let y = geometry.origin_y + geometry.grid_height + 1;
            buffer.set_string(x, y, label.to_string(), label_style);
        }
    }
    fn render_moves_panel(&self, frame: &mut Frame<'_>, area: Rect) {
        let inner = self.render_panel(frame, area);

        let Some(replay) = self.current_replay() else {
            return;
        };
        let full_moves = replay.san_moves.len().div_ceil(2);
        let mut lines: Vec<Line<'_>> = Vec::with_capacity(full_moves);
        for pair_index in 0..full_moves {
            let number = pair_index + 1;
            let white_index = 2 * pair_index;
            let black_index = 2 * pair_index + 1;

            let mut spans = vec![Span::styled(
                format!("{number:>3}. "),
                Style::default().fg(self.theme.move_number),
            )];
            spans.push(self.move_cell(replay, white_index));
            if black_index < replay.san_moves.len() {
                spans.push(Span::raw(" "));
                spans.push(self.move_cell(replay, black_index));
            }
            lines.push(Line::from(spans));
        }

        // Keep the current move on screen.
        let current_row = if self.ply == 0 { 0 } else { (self.ply - 1) / 2 } as u16;
        let visible = inner.height;
        let scroll = if visible > 0 && current_row >= visible {
            current_row - visible + 1
        } else {
            0
        };

        let paragraph = Paragraph::new(lines).scroll((scroll, 0));
        frame.render_widget(paragraph, inner);
    }

    /// A single move token in the list, highlighted if it is the current move.
    fn move_cell(&self, replay: &Replay, half_move_index: usize) -> Span<'_> {
        let text = format!(
            "{:<width$}",
            replay.san_moves[half_move_index],
            width = MOVE_CELL_WIDTH
        );
        let is_current = self.ply > 0 && self.ply - 1 == half_move_index;
        let style = if is_current {
            Style::default()
                .bg(self.theme.current_move_background)
                .fg(self.theme.current_move_foreground)
        } else {
            Style::default().fg(self.theme.move_text)
        };
        Span::styled(text, style)
    }

    // -- run loop ----------------------------------------------------------

    fn event_loop(&mut self, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
        loop {
            terminal
                .draw(|frame| self.render(frame))
                .into_diagnostic()?;
            if self.should_quit {
                break;
            }

            // Block for one event, then drain whatever else is already queued
            // before drawing again. Dragging a window edge produces a burst of
            // resize events, and redrawing for every intermediate size means
            // rebuilding every piece image each time. Every event is still
            // handled — only the redraws are coalesced — so no key press is
            // dropped.
            let mut event = event::read().into_diagnostic()?;
            loop {
                self.handle_event(&event);
                if self.should_quit || !event::poll(Duration::ZERO).into_diagnostic()? {
                    break;
                }
                event = event::read().into_diagnostic()?;
            }
        }
        Ok(())
    }

    fn handle_event(&mut self, event: &Event) {
        match event {
            Event::Key(key) if key.kind == KeyEventKind::Press => self.handle_key(key.code),
            Event::Mouse(mouse) => self.handle_mouse(mouse.kind, mouse.column, mouse.row),
            _ => {}
        }
    }
}

/// Where the board grid sits and how big each square is.
struct BoardGeometry {
    origin_x: u16,
    origin_y: u16,
    cell_width: u16,
    cell_height: u16,
    grid_height: u16,
}

impl BoardGeometry {
    /// The rectangle of one square. `display_row` is measured from the top
    /// (rank 8), matching how the position is stored.
    fn cell(&self, display_row: u16, file: u16) -> Rect {
        Rect::new(
            self.origin_x + file * self.cell_width,
            self.origin_y + display_row * self.cell_height,
            self.cell_width,
            self.cell_height,
        )
    }
}

/// The rectangles of the menu screen.
struct MenuRegions {
    list: Rect,
    bottom: Rect,
}

fn menu_layout(area: Rect) -> MenuRegions {
    let rows =
        Layout::vertical([Constraint::Min(0), Constraint::Length(BOTTOM_BAR_HEIGHT)]).split(area);
    MenuRegions {
        list: rows[0],
        bottom: rows[1],
    }
}

/// The rectangles of the board screen.
struct BoardRegions {
    header: Rect,
    board: Rect,
    moves: Rect,
    buttons: [Rect; 4],
}

fn board_layout(area: Rect) -> BoardRegions {
    let rows = Layout::vertical([
        Constraint::Length(HEADER_HEIGHT),
        Constraint::Min(0),
        Constraint::Length(BOTTOM_BAR_HEIGHT),
    ])
    .split(area);

    let columns = Layout::horizontal([
        Constraint::Min(BOARD_MIN_WIDTH),
        Constraint::Length(MOVES_PANEL_WIDTH),
    ])
    .split(rows[1]);

    let buttons = Layout::horizontal([
        Constraint::Ratio(1, 4),
        Constraint::Ratio(1, 4),
        Constraint::Ratio(1, 4),
        Constraint::Ratio(1, 4),
    ])
    .split(rows[2]);

    BoardRegions {
        header: rows[0],
        board: columns[0],
        moves: columns[1],
        buttons: [buttons[0], buttons[1], buttons[2], buttons[3]],
    }
}

/// Draws one bordered bar along the bottom: a button, a hint, or a problem.
fn render_bottom_text(
    frame: &mut Frame<'_>,
    area: Rect,
    text: &str,
    foreground: Color,
    theme: Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.panel_border))
        .style(Style::default().bg(theme.button_background));
    let paragraph = Paragraph::new(text)
        .alignment(Alignment::Center)
        .style(Style::default().fg(foreground))
        .block(block);
    frame.render_widget(paragraph, area);
}

fn render_button(frame: &mut Frame<'_>, area: Rect, label: &str, theme: Theme) {
    render_bottom_text(frame, area, label, theme.button_foreground, theme);
}

/// Truncates to `width` and pads to it, so a highlighted row fills its panel.
/// A name cut short ends in an ellipsis, so that a truncated name cannot be
/// mistaken for a short one.
fn fit(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let length = text.chars().count();
    if length > width {
        let mut fitted: String = text.chars().take(width - 1).collect();
        fitted.push('…');
        return fitted;
    }
    // `width >= length` here, so this pads on the right without truncating.
    format!("{text:<width$}")
}

fn panel_block(theme: Theme) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.panel_border))
        .style(Style::default().bg(theme.panel_background))
}

/// The board character at a display row and file, or `.` (an empty square) when
/// the position has a short or missing row. Every square lookup goes through
/// here, so no indexing can run past the end of a row and panic mid-draw.
fn cell_char(rows: &[&[u8]; 8], display_row: u16, file: u16) -> char {
    rows.get(display_row as usize)
        .and_then(|row| row.get(file as usize))
        .copied()
        .unwrap_or(b'.') as char
}

/// Draws a piece into a board cell, using the block sprites when the cell is
/// big enough and falling back to a single glyph on very small terminals.
fn draw_piece(
    buffer: &mut Buffer,
    cell: Rect,
    character: char,
    foreground: Color,
    background: Color,
) {
    let style = Style::default().fg(foreground).bg(background);
    if cell.width >= 6 && cell.height >= 3 {
        draw_block_piece(buffer, cell, character, style);
    } else {
        draw_glyph(buffer, cell, glyph_for(character), style);
    }
}

fn draw_glyph(buffer: &mut Buffer, cell: Rect, glyph: &str, style: Style) {
    buffer.set_string(
        cell.x + cell.width / 2,
        cell.y + cell.height / 2,
        glyph,
        style,
    );
}

/// The side, in pixels, of the sprite bitmaps.
const SPRITE_SIDE: usize = 6;

/// Draws a piece as a half-block silhouette, scaled up to the square.
///
/// Each terminal cell carries two vertical pixels, so a cell of `w` columns and
/// `h` rows is a `w` by `2h` pixel canvas. The bitmap is scaled by a whole
/// number and centered in it: whole numbers only, because stretching a 6-pixel
/// sprite by a fraction would duplicate some rows and not others, which
/// visibly breaks the symmetry of pieces like the king and the queen.
///
/// A board square is always twice as many columns as rows, so the canvas is
/// even on both axes; the sprite is an even number of pixels too, which leaves
/// an even margin and lets the piece sit exactly in the middle.
fn draw_block_piece(buffer: &mut Buffer, cell: Rect, character: char, style: Style) {
    let sprite = block_sprite(character.to_ascii_uppercase());
    let canvas_width = cell.width as usize;
    let canvas_height = cell.height as usize * 2;

    let scale = (canvas_width.min(canvas_height) / SPRITE_SIDE).max(1);
    let side = SPRITE_SIDE * scale;
    if side > canvas_width || side > canvas_height {
        return;
    }
    let offset_x = (canvas_width - side) / 2;
    let offset_y = (canvas_height - side) / 2;

    let lit = |x: usize, y: usize| -> bool {
        let (Some(sprite_x), Some(sprite_y)) = (x.checked_sub(offset_x), y.checked_sub(offset_y))
        else {
            return false;
        };
        if sprite_x >= side || sprite_y >= side {
            return false;
        }
        sprite[sprite_y / scale].as_bytes()[sprite_x / scale] == b'X'
    };

    for row in 0..cell.height as usize {
        let mut line = String::with_capacity(canvas_width);
        for column in 0..canvas_width {
            line.push(match (lit(column, row * 2), lit(column, row * 2 + 1)) {
                (true, true) => '\u{2588}',  // full block
                (true, false) => '\u{2580}', // upper half
                (false, true) => '\u{2584}', // lower half
                (false, false) => ' ',
            });
        }
        buffer.set_string(cell.x, cell.y + row as u16, line, style);
    }
}

/// A 6x6-pixel silhouette per piece, keyed by uppercase letter. `X` is a piece
/// pixel, anything else is transparent (shows the square colour).
fn block_sprite(kind_upper: char) -> [&'static str; 6] {
    match kind_upper {
        'K' => [
            "..XX..", // cross: vertical
            "XXXXXX", // cross: wide arms
            "..XX..", // narrow stem below the cross
            "..XX..", ".XXXX.", "XXXXXX",
        ],
        'Q' => [
            "X.X.X.", // three separated crown spikes
            "X.X.X.", "XXXXXX", // crown band
            ".XXXX.", ".XXXX.", "XXXXXX",
        ],
        'R' => [
            "X.XX.X", // crenellations
            "XXXXXX", ".XXXX.", ".XXXX.", ".XXXX.", "XXXXXX",
        ],
        'B' => [
            "..XX..", // rounded bulb head
            ".XXXX.", ".XXXX.", "..XX..", // shoulders pinch to a neck
            ".XXXX.", "XXXXXX",
        ],
        'N' => [
            ".XXX..", // ears / head
            "XXXXX.", "XX.XX.", // muzzle notch
            "..XXX.", ".XXXX.", "XXXXXX",
        ],
        'P' => ["..XX..", ".XXXX.", "..XX..", "..XX..", ".XXXX.", "XXXXXX"],
        _ => ["......", "......", "......", "......", "......", "......"],
    }
}

/// Maps an ascii board character to the glyph drawn for it.
fn glyph_for(character: char) -> &'static str {
    match character {
        'K' => "\u{2654}",
        'Q' => "\u{2655}",
        'R' => "\u{2656}",
        'B' => "\u{2657}",
        'N' => "\u{2658}",
        'P' => "\u{2659}",
        'k' => "\u{265A}",
        'q' => "\u{265B}",
        'r' => "\u{265C}",
        'b' => "\u{265D}",
        'n' => "\u{265E}",
        'p' => "\u{265F}",
        _ => " ",
    }
}

fn hit(rect: Rect, column: u16, row: u16) -> bool {
    column >= rect.x && column < rect.x + rect.width && row >= rect.y && row < rect.y + rect.height
}
// ===========================================================================
// ENTRY POINT AND FILE LOADING
// ===========================================================================

/// Loads the game, runs the terminal UI, and restores the terminal afterwards.
///
/// The PGN comes from a path given on the command line (`chessview <file.pgn>`)
/// when one is present, otherwise from the single file in `<data_dir>/chessview/`.
///
/// # Errors
///
/// Returns an error if a path given on the command line cannot be read or is not
/// valid PGN, if more than one path is given, or if the terminal cannot be set
/// up or restored. A *missing* default file is not an error: the app starts in
/// the menu with an explanatory message instead.
pub fn run() -> Result<()> {
    let mut app = load_app(std::env::args_os().skip(1))?;

    // Query for a graphics protocol before entering the alternate screen.
    #[cfg(feature = "images")]
    app.enable_images();

    let mut terminal = setup_terminal()?;
    let result = app.event_loop(&mut terminal);
    restore_terminal(&mut terminal)?;
    result
}

/// Chooses where the PGN comes from and builds the app.
///
/// A path given in `args` is opened directly, and any problem with it (missing,
/// unreadable, or invalid PGN) is a hard error. With no path, the default file
/// is used; if that cannot be loaded or parsed the app still starts, showing an
/// explanation in the menu rather than exiting.
///
/// `args` is the program arguments with the executable name already removed.
///
/// # Errors
///
/// Returns an error only for a path given in `args` (see above) or when more
/// than one path is given.
fn load_app<I: Iterator<Item = OsString>>(args: I) -> Result<App> {
    if let Some(path) = pgn_path_argument(args)? {
        let text = read_pgn_file(&path)?;
        return App::from_pgn(&text);
    }
    let app = match load_default_pgn_text() {
        Ok(text) => {
            App::from_pgn(&text).unwrap_or_else(|error| App::empty(no_file_message(&error)))
        }
        Err(error) => App::empty(no_file_message(&error)),
    };
    Ok(app)
}

/// The message shown in the menu when no game could be loaded without an
/// explicit file argument.
fn no_file_message(reason: &miette::Report) -> String {
    format!("{reason}. Open a PGN file with: chessview <file.pgn>")
}

/// The PGN path given on the command line, if any. `args` is the program
/// arguments with the executable name already removed.
///
/// # Errors
///
/// Returns an error if more than one path is given.
fn pgn_path_argument<I: Iterator<Item = OsString>>(args: I) -> Result<Option<PathBuf>> {
    let mut paths = args.map(PathBuf::from);
    match (paths.next(), paths.next()) {
        (None, _) => Ok(None),
        (Some(path), None) => Ok(Some(path)),
        (Some(_), Some(_)) => Err(miette!(
            "expected at most one PGN file; usage: chessview <file.pgn>"
        )),
    }
}

/// Reads a PGN file chosen on the command line.
fn read_pgn_file(path: &Path) -> Result<String> {
    std::fs::read_to_string(path)
        .into_diagnostic()
        .wrap_err_with(|| format!("could not read {}", path.display()))
}

/// Reads the single PGN file from `<data_dir>/chessview/`, erroring if the
/// directory holds zero or more than one file.
fn load_default_pgn_text() -> Result<String> {
    let base =
        dirs::data_dir().ok_or_else(|| miette!("could not determine the OS data directory"))?;
    read_single_pgn(&base.join("chessview"))
}

/// Reads the single PGN file in `directory`, erroring if it holds zero or more
/// than one file. Split from [`load_default_pgn_text`] so the zero/one/many
/// logic can be exercised against a temporary directory.
fn read_single_pgn(directory: &Path) -> Result<String> {
    let entries = std::fs::read_dir(directory)
        .into_diagnostic()
        .wrap_err_with(|| format!("could not read {}", directory.display()))?;

    let mut files: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let entry = entry.into_diagnostic()?;
        if entry.path().is_file() {
            files.push(entry.path());
        }
    }

    match files.as_slice() {
        [] => Err(miette!("no PGN file found in {}", directory.display())),
        [only] => std::fs::read_to_string(only)
            .into_diagnostic()
            .wrap_err_with(|| format!("could not read {}", only.display())),
        many => Err(miette!(
            "expected exactly one file in {}, found {}",
            directory.display(),
            many.len()
        )),
    }
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode().into_diagnostic()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture).into_diagnostic()?;
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend).into_diagnostic()
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    disable_raw_mode().into_diagnostic()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )
    .into_diagnostic()?;
    terminal.show_cursor().into_diagnostic()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::KeyEvent;
    use ratatui::crossterm::event::KeyModifiers;

    // =======================================================================
    // Test harness
    //
    // Tests talk to the app through `Harness` and `Screen` rather than calling
    // its methods directly, so that a change to the app's own API ripples into
    // this one place instead of into every test.
    // =======================================================================

    const SAMPLE: &str = "1. e4 e5 2. Nf3 Nc6 3. Bb5 a6";
    const TWO_GAMES: &str = "\
[White \"Alice\"] [Black \"Bob\"] 1. e4 e5 1-0
[Event \"Second\"] 1. d4 d5 2. c4 0-1
";
    /// Two games, neither named.
    const TWO_UNNAMED: &str = "1. e4 e5 1-0 1. d4 d5 0-1";

    /// The size used when a test needs a screen but does not care how big.
    const DEFAULT_SIZE: (u16, u16) = (90, 30);

    /// Whether building an app from this PGN is rejected.
    fn is_rejected(pgn: &str) -> bool {
        App::from_pgn(pgn).is_err()
    }

    /// A driver around one `App`. Action methods return `&mut Self` so steps can
    /// be chained; query methods read state back out.
    struct Harness {
        app: App,
    }

    impl Harness {
        // -- construction --------------------------------------------------

        fn new(pgn: &str) -> Harness {
            Harness {
                app: App::from_pgn(pgn).unwrap(),
            }
        }

        fn empty(message: &str) -> Harness {
            Harness {
                app: App::empty(message),
            }
        }

        // -- actions -------------------------------------------------------

        fn press(&mut self, code: KeyCode) -> &mut Self {
            self.app.handle_key(code);
            self
        }

        /// Sends a raw key event, so tests can exercise the press/release
        /// filtering that `press` (a plain key press) hides.
        fn send_key(&mut self, code: KeyCode, kind: KeyEventKind) -> &mut Self {
            let event = Event::Key(KeyEvent::new_with_kind(code, KeyModifiers::NONE, kind));
            self.app.handle_event(&event);
            self
        }

        fn mouse(&mut self, kind: MouseEventKind, x: u16, y: u16) -> &mut Self {
            self.app.handle_mouse(kind, x, y);
            self
        }

        fn open(&mut self, index: usize) -> &mut Self {
            self.app.open_game(index);
            self
        }

        fn back_to_menu(&mut self) -> &mut Self {
            self.app.show_menu();
            self
        }

        fn forward(&mut self) -> &mut Self {
            self.app.forward();
            self
        }

        fn backward(&mut self) -> &mut Self {
            self.app.backward();
            self
        }

        fn reset(&mut self) -> &mut Self {
            self.app.reset();
            self
        }

        fn go_to_end(&mut self) -> &mut Self {
            self.app.go_to_end();
            self
        }

        fn set_ply(&mut self, ply: usize) -> &mut Self {
            self.app.ply = ply;
            self
        }

        fn select(&mut self, index: usize) -> &mut Self {
            self.app.selected = index;
            self
        }

        /// Left-clicks a menu row (0 is the top visible row).
        fn click_menu_row(&mut self, row: u16) -> &mut Self {
            let (x, y) = self.menu_row_xy(row);
            self.mouse(MouseEventKind::Down(MouseButton::Left), x, y)
        }

        /// Left-clicks one of the four board buttons.
        fn click_button(&mut self, index: usize) -> &mut Self {
            self.ensure_rendered();
            let button = board_layout(self.app.viewport).buttons[index];
            self.mouse(
                MouseEventKind::Down(MouseButton::Left),
                button.x + 1,
                button.y + 1,
            )
        }

        /// The screen coordinates of a menu row, for tests that drive the mouse
        /// directly (for instance to check a non-left button does nothing).
        fn menu_row_xy(&mut self, row: u16) -> (u16, u16) {
            self.ensure_rendered();
            let inner = panel_block(self.app.theme).inner(menu_layout(self.app.viewport).list);
            (inner.x + 2, inner.y + row)
        }

        // -- rendering -----------------------------------------------------

        /// Renders at `width`x`height` and returns the resulting screen. This
        /// also updates the viewport used for mouse hit-testing.
        fn render(&mut self, width: u16, height: u16) -> Screen {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| self.app.render(frame)).unwrap();
            Screen {
                buffer: terminal.backend().buffer().clone(),
                viewport: self.app.viewport,
            }
        }

        /// Renders once at the default size if nothing has been rendered yet, so
        /// mouse helpers have a viewport to hit-test against.
        fn ensure_rendered(&mut self) {
            if self.app.viewport.width == 0 || self.app.viewport.height == 0 {
                let _ = self.render(DEFAULT_SIZE.0, DEFAULT_SIZE.1);
            }
        }

        // -- queries -------------------------------------------------------

        fn mode(&self) -> Mode {
            self.app.mode
        }

        fn ply(&self) -> usize {
            self.app.ply
        }

        fn selected(&self) -> usize {
            self.app.selected
        }

        fn current_game(&self) -> usize {
            self.app.current_game
        }

        fn should_quit(&self) -> bool {
            self.app.should_quit
        }

        fn status(&self) -> Option<&str> {
            self.app.status.as_deref()
        }

        fn game_count(&self) -> usize {
            self.app.games.len()
        }

        fn game_name(&self, index: usize) -> &str {
            &self.app.games[index].name
        }

        fn game_names(&self) -> Vec<&str> {
            self.app
                .games
                .iter()
                .map(|game| game.name.as_str())
                .collect()
        }

        fn is_replayed(&self, index: usize) -> bool {
            self.app.games[index].replay.is_some()
        }

        /// The number of stored positions for an opened game (moves + 1).
        fn position_count(&self, index: usize) -> usize {
            self.app.games[index]
                .replay
                .as_ref()
                .unwrap()
                .positions
                .len()
        }

        fn action_count(&self) -> usize {
            self.app.current_actions().len()
        }

        fn highlights(&self) -> Vec<(u8, u8)> {
            self.app.last_move_highlights()
        }

        /// Highlights in a stable order, for moves that yield more than one.
        fn sorted_highlights(&self) -> Vec<(u8, u8)> {
            let mut highlights = self.highlights();
            highlights.sort_unstable();
            highlights
        }

        fn menu_scroll(&self, visible: u16) -> u16 {
            self.app.menu_scroll(visible)
        }
    }

    /// A rendered screen. Region accessors return just the text of one panel so
    /// an assertion cannot pass on a match from an unrelated part of the screen.
    struct Screen {
        buffer: Buffer,
        viewport: Rect,
    }

    impl Screen {
        /// The whole screen as text, one line per row.
        fn text(&self) -> String {
            region_text(&self.buffer, self.buffer.area)
        }

        fn menu_list(&self) -> String {
            region_text(&self.buffer, menu_layout(self.viewport).list)
        }

        fn menu_bottom(&self) -> String {
            region_text(&self.buffer, menu_layout(self.viewport).bottom)
        }

        fn header(&self) -> String {
            region_text(&self.buffer, board_layout(self.viewport).header)
        }

        fn moves(&self) -> String {
            region_text(&self.buffer, board_layout(self.viewport).moves)
        }

        fn button(&self, index: usize) -> String {
            region_text(&self.buffer, board_layout(self.viewport).buttons[index])
        }

        /// The board, if one was drawn.
        fn board(&self) -> Option<Board<'_>> {
            Board::locate(&self.buffer)
        }
    }

    /// The text inside `area`, one line per row.
    fn region_text(buffer: &Buffer, area: Rect) -> String {
        let mut text = String::new();
        for y in area.y..area.y.saturating_add(area.height) {
            for x in area.x..area.x.saturating_add(area.width) {
                text.push_str(buffer[(x, y)].symbol());
            }
            text.push('\n');
        }
        text
    }

    fn is_board_square(color: Color) -> bool {
        color == DEFAULT_THEME.light_square
            || color == DEFAULT_THEME.dark_square
            || color == DEFAULT_THEME.light_square_last_move
            || color == DEFAULT_THEME.dark_square_last_move
    }

    /// The piece ink on a square, as the blank margins inside it (in cells) plus
    /// the ink width. Equal margins mean the piece is centered.
    struct InkBounds {
        leading: u16,
        trailing: u16,
        above: u16,
        below: u16,
        width: u16,
    }

    /// The board located in a rendered buffer, addressed in files and ranks.
    struct Board<'a> {
        buffer: &'a Buffer,
        left: u16,
        top: u16,
        cell_width: u16,
        cell_height: u16,
    }

    impl<'a> Board<'a> {
        /// Locates the board by its square background colours, or `None` when no
        /// board was drawn (for instance a terminal too small to hold one).
        fn locate(buffer: &'a Buffer) -> Option<Board<'a>> {
            let mut columns = Vec::new();
            let mut rows = Vec::new();
            for y in 0..buffer.area.height {
                for x in 0..buffer.area.width {
                    if is_board_square(buffer[(x, y)].bg) {
                        columns.push(x);
                        rows.push(y);
                    }
                }
            }
            let left = *columns.iter().min()?;
            let top = *rows.iter().min()?;
            let cell_width = (columns.iter().max()? - left + 1) / 8;
            let cell_height = (rows.iter().max()? - top + 1) / 8;
            Some(Board {
                buffer,
                left,
                top,
                cell_width,
                cell_height,
            })
        }

        /// The rectangle of a square, addressed as a player names it: `file` a..h
        /// as 0..7 and `rank` 1..8 as 0..7.
        fn square(&self, file: u16, rank: u16) -> Rect {
            Rect::new(
                self.left + file * self.cell_width,
                self.top + (7 - rank) * self.cell_height,
                self.cell_width,
                self.cell_height,
            )
        }

        /// The ink on a square, or `None` when the square is empty. Ink is any
        /// cell with a non-blank glyph and a real (non-`Reset`) foreground.
        fn ink(&self, file: u16, rank: u16) -> Option<InkBounds> {
            let square = self.square(file, rank);
            let mut xs = Vec::new();
            let mut ys = Vec::new();
            for y in square.y..square.y + square.height {
                for x in square.x..square.x + square.width {
                    let cell = &self.buffer[(x, y)];
                    if cell.symbol() != " " && cell.fg != Color::Reset {
                        xs.push(x);
                        ys.push(y);
                    }
                }
            }
            let min_x = *xs.iter().min()?;
            let max_x = *xs.iter().max()?;
            let min_y = *ys.iter().min()?;
            let max_y = *ys.iter().max()?;
            Some(InkBounds {
                leading: min_x - square.x,
                trailing: square.x + square.width - 1 - max_x,
                above: min_y - square.y,
                below: square.y + square.height - 1 - max_y,
                width: max_x - min_x + 1,
            })
        }
    }

    // =======================================================================
    // Tests
    // =======================================================================

    mod opening {
        use super::*;

        #[test]
        fn a_single_game_opens_straight_onto_the_board() {
            let harness = Harness::new(SAMPLE);
            assert_eq!(harness.mode(), Mode::Board);
            assert_eq!(harness.game_count(), 1);
            // Opening a game builds its replay: seven positions for six half-moves.
            assert_eq!(harness.position_count(0), 7);
        }

        #[test]
        fn several_games_open_the_menu() {
            let harness = Harness::new(TWO_GAMES);
            assert_eq!(harness.mode(), Mode::Menu);
            assert_eq!(harness.game_count(), 2);
        }

        #[test]
        fn pgn_without_games_is_rejected() {
            assert!(is_rejected(""));
            assert!(is_rejected("   "));
        }

        #[test]
        fn invalid_pgn_is_rejected() {
            assert!(is_rejected("1. e4 Zx9"));
        }
    }

    mod game_names {
        use super::*;

        #[test]
        fn named_games_keep_their_name() {
            let harness = Harness::new(TWO_GAMES);
            // `1-0` here is a result *token*, not a `[Result "1-0"]` tag, and
            // names are built from tag pairs only.
            assert_eq!(harness.game_name(0), "Alice vs Bob");
            assert_eq!(harness.game_name(1), "Second");
        }

        #[test]
        fn unnamed_games_are_numbered_by_file_order() {
            let harness = Harness::new(TWO_UNNAMED);
            assert_eq!(harness.game_name(0), "Game 1");
            assert_eq!(harness.game_name(1), "Game 2");
        }

        /// Numbering follows position in the file, so a named game does not
        /// shift the numbers of the others.
        #[test]
        fn numbering_is_independent_of_which_games_are_named() {
            let harness = Harness::new("1. e4 1-0 [White \"Alice\"] 1. d4 0-1 1. c4 1/2-1/2");
            assert_eq!(harness.game_names(), vec!["Game 1", "Alice", "Game 3"]);
        }
    }

    mod lazy_replay {
        use super::*;

        #[test]
        fn games_are_replayed_only_when_opened() {
            let mut harness = Harness::new(TWO_GAMES);
            assert!(!harness.is_replayed(0) && !harness.is_replayed(1));

            harness.open(1);
            assert!(!harness.is_replayed(0), "untouched game stays unbuilt");
            assert!(harness.is_replayed(1));
        }

        /// One unplayable game must not stop the file from opening, or take the
        /// app down when picked: the menu stays up and says what happened.
        #[test]
        fn an_unplayable_game_reports_instead_of_crashing() {
            // The second game's rook cannot reach a3 from the starting position.
            let mut harness = Harness::new("1. e4 e5 1-0 1. Ra3 0-1");
            assert_eq!(harness.game_count(), 2);

            harness.open(1);
            assert_eq!(harness.mode(), Mode::Menu, "stays in the menu");
            assert!(harness.status().is_some(), "reports the problem");

            // The healthy game still opens, and clears the message.
            harness.open(0);
            assert_eq!(harness.mode(), Mode::Board);
            assert!(harness.status().is_none());
        }
    }

    mod menu_navigation {
        use super::*;

        #[test]
        fn menu_selection_stays_in_bounds() {
            let mut harness = Harness::new(TWO_GAMES);
            assert_eq!(harness.selected(), 0);
            harness.press(KeyCode::Up);
            assert_eq!(harness.selected(), 0, "cannot move above the first game");
            for _ in 0..5 {
                harness.press(KeyCode::Down);
            }
            assert_eq!(harness.selected(), 1, "cannot move past the last game");
            harness.press(KeyCode::Home);
            assert_eq!(harness.selected(), 0);
            harness.press(KeyCode::End);
            assert_eq!(harness.selected(), 1);
        }

        #[test]
        fn enter_opens_the_selected_game() {
            let mut harness = Harness::new(TWO_GAMES);
            harness.press(KeyCode::Down);
            harness.press(KeyCode::Enter);
            assert_eq!(harness.mode(), Mode::Board);
            assert_eq!(harness.current_game(), 1);
            assert_eq!(harness.ply(), 0);
        }

        #[test]
        fn opening_a_game_starts_it_at_the_first_position() {
            let mut harness = Harness::new(TWO_GAMES);
            harness.open(0);
            harness.forward();
            assert_eq!(harness.ply(), 1);
            harness.back_to_menu();
            harness.open(1);
            assert_eq!(harness.ply(), 0, "the new game starts from the beginning");
        }
    }

    mod leaving_a_game {
        use super::*;

        #[test]
        fn escape_and_a_return_to_the_menu() {
            for key in [KeyCode::Esc, KeyCode::Char('a')] {
                let mut harness = Harness::new(TWO_GAMES);
                harness.open(1);
                assert_eq!(harness.mode(), Mode::Board);
                harness.press(key);
                assert_eq!(harness.mode(), Mode::Menu);
                assert_eq!(harness.selected(), 1, "the game just viewed is selected");
            }
        }

        /// A single game skips the menu, but the menu is still reachable.
        #[test]
        fn a_single_game_can_still_reach_the_menu() {
            let mut harness = Harness::new(SAMPLE);
            assert_eq!(harness.mode(), Mode::Board);
            harness.press(KeyCode::Esc);
            assert_eq!(harness.mode(), Mode::Menu);
        }

        #[test]
        fn q_quits_from_either_mode() {
            let mut board = Harness::new(SAMPLE);
            board.press(KeyCode::Char('q'));
            assert!(board.should_quit());

            let mut menu = Harness::new(TWO_GAMES);
            menu.press(KeyCode::Char('q'));
            assert!(menu.should_quit());
        }

        /// Escape quits from the menu — there is nowhere further back to go.
        #[test]
        fn escape_quits_from_the_menu() {
            let mut harness = Harness::new(TWO_GAMES);
            harness.press(KeyCode::Esc);
            assert!(harness.should_quit());
        }
    }

    mod board_navigation {
        use super::*;

        #[test]
        fn board_navigation_stays_in_bounds() {
            let mut harness = Harness::new(SAMPLE);
            assert_eq!(harness.ply(), 0);
            harness.backward();
            assert_eq!(harness.ply(), 0, "cannot go before the start");
            for _ in 0..10 {
                harness.forward();
            }
            assert_eq!(harness.ply(), 6, "cannot go past the end");
            harness.reset();
            assert_eq!(harness.ply(), 0);
            harness.go_to_end();
            assert_eq!(harness.ply(), 6);
        }

        #[test]
        fn navigation_follows_the_open_game() {
            let mut harness = Harness::new(TWO_GAMES);
            harness.open(1); // 1. d4 d5 2. c4 -> three half-moves
            harness.go_to_end();
            assert_eq!(harness.ply(), 3);
        }

        /// The board key bindings are wired to the navigation methods, which are
        /// themselves covered by `board_navigation_stays_in_bounds`.
        #[test]
        fn board_keys_drive_navigation() {
            let mut harness = Harness::new(SAMPLE); // six half-moves
            assert_eq!(harness.mode(), Mode::Board);
            harness.press(KeyCode::Right);
            assert_eq!(harness.ply(), 1, "Right steps forward");
            harness.press(KeyCode::Right);
            harness.press(KeyCode::Left);
            assert_eq!(harness.ply(), 1, "Left steps back");
            harness.press(KeyCode::End);
            assert_eq!(harness.ply(), 6, "End jumps to the last position");
            harness.press(KeyCode::Char('r'));
            assert_eq!(harness.ply(), 0, "r resets to the start");
            harness.press(KeyCode::End);
            harness.press(KeyCode::Home);
            assert_eq!(harness.ply(), 0, "Home also resets");
        }

        #[test]
        fn the_move_buttons_navigate() {
            let mut harness = Harness::new(SAMPLE);
            harness.click_button(3); // forward
            assert_eq!(harness.ply(), 1);
            harness.click_button(3);
            assert_eq!(harness.ply(), 2);
            harness.click_button(1); // back
            assert_eq!(harness.ply(), 1);
            harness.click_button(2); // reset
            assert_eq!(harness.ply(), 0);
        }
    }

    mod last_move_highlight {
        use super::*;

        #[test]
        fn tracks_the_destination_of_a_plain_move() {
            let mut harness = Harness::new(SAMPLE);
            harness.forward(); // 1. e4
            assert_eq!(harness.highlights(), vec![(4, 3)]); // e4
        }

        /// Castling is the only move whose highlight depends on knowing *whose*
        /// move it was, so it pins down the white/black half-move parity.
        #[test]
        fn castling_highlights_the_moving_side_back_rank() {
            let mut harness = Harness::new("1. e4 e5 2. Nf3 Nc6 3. Bc4 Bc5 4. O-O Nf6 5. d3 O-O");

            harness.set_ply(7); // white's O-O: king g1, rook f1 on rank 1 (index 0)
            assert_eq!(harness.sorted_highlights(), vec![(5, 0), (6, 0)]);

            harness.set_ply(10); // black's O-O: king g8, rook f8 on rank 8 (index 7)
            assert_eq!(harness.sorted_highlights(), vec![(5, 7), (6, 7)]);
        }

        #[test]
        fn after_a_capture() {
            let mut harness = Harness::new("1. e4 d5 2. exd5");
            harness.go_to_end(); // white's exd5
            assert_eq!(harness.ply(), 3);
            assert_eq!(harness.highlights(), vec![(3, 4)]); // d5
        }

        #[test]
        fn after_en_passant() {
            let mut harness = Harness::new("1. e4 d5 2. e5 f5 3. exf6");
            harness.go_to_end(); // white's en passant exf6
            assert_eq!(harness.ply(), 5);
            assert_eq!(harness.highlights(), vec![(5, 5)]); // f6
        }

        #[test]
        fn after_a_promotion() {
            let mut harness = Harness::new("1. h4 g5 2. hxg5 a6 3. g6 a5 4. g7 a4 5. gxh8=Q");
            harness.go_to_end(); // white's gxh8=Q
            assert_eq!(harness.ply(), 9);
            assert_eq!(harness.highlights(), vec![(7, 7)]); // h8
        }

        /// The queenside castle highlight is a separate branch from the
        /// kingside one and lands the king and rook on the c- and d-files.
        #[test]
        fn queenside_castling_highlights_the_back_rank() {
            let mut harness =
                Harness::new("1. d4 d5 2. Nc3 Nc6 3. Bf4 Bf5 4. Qd2 Qd7 5. O-O-O O-O-O");

            harness.set_ply(9); // white's O-O-O: king c1, rook d1 on rank 1 (index 0)
            assert_eq!(harness.sorted_highlights(), vec![(2, 0), (3, 0)]);

            harness.set_ply(10); // black's O-O-O: king c8, rook d8 on rank 8 (index 7)
            assert_eq!(harness.sorted_highlights(), vec![(2, 7), (3, 7)]);
        }

        #[test]
        fn nothing_is_highlighted_before_the_first_move() {
            let harness = Harness::new(SAMPLE); // opens at ply 0
            assert!(harness.highlights().is_empty());
        }
    }

    mod rendering {
        use super::*;

        #[test]
        fn renders_both_modes_without_panicking() {
            let mut harness = Harness::new(TWO_GAMES);
            harness.render(90, 30); // menu
            harness.open(0);
            for _ in 0..=harness.action_count() {
                harness.render(90, 30); // board
                harness.forward();
            }
        }

        #[test]
        fn renders_in_a_tiny_terminal_without_panicking() {
            let mut harness = Harness::new(TWO_GAMES);
            for (width, height) in [(1u16, 1u16), (10, 3), (20, 8), (5, 40)] {
                harness.render(width, height);
                harness.open(0);
                harness.render(width, height);
                harness.back_to_menu();
            }
        }

        #[test]
        fn the_menu_lists_every_game() {
            let screen = Harness::new(TWO_GAMES).render(90, 20);
            assert!(
                screen.menu_list().contains("Alice vs Bob"),
                "first game listed"
            );
            assert!(screen.menu_list().contains("Second"), "second game listed");
            assert!(screen.menu_bottom().contains("Enter"), "hint bar shown");
        }

        #[test]
        fn the_menu_reports_an_unplayable_game() {
            let mut harness = Harness::new("1. e4 e5 1-0 1. Ra3 0-1");
            harness.open(1);
            let screen = harness.render(90, 20);
            assert!(
                screen.menu_bottom().contains("could not apply move"),
                "problem shown: {}",
                screen.menu_bottom()
            );
        }

        #[test]
        fn the_board_shows_the_game_name_and_moves() {
            let mut harness = Harness::new(TWO_GAMES);
            harness.open(0);
            harness.set_ply(2); // "1. e4 e5" is two half-moves
            let screen = harness.render(90, 30);
            assert!(
                screen.header().contains("Alice vs Bob"),
                "game name shown in header"
            );
            assert!(
                screen.moves().contains("e4") && screen.moves().contains("e5"),
                "moves shown in the move list"
            );
            assert!(
                screen.button(0).contains("All games"),
                "route back to the menu shown"
            );
        }

        /// A name too long for its panel is cut with an ellipsis, so it cannot
        /// be mistaken for a short name.
        #[test]
        fn long_names_are_truncated_with_an_ellipsis() {
            let long =
                "[White \"Aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"] 1. e4 1-0 \
                        1. d4 1-0";
            let mut harness = Harness::new(long);
            assert!(
                harness.render(30, 10).menu_list().contains('…'),
                "menu name truncated"
            );

            harness.open(0);
            assert!(
                harness.render(30, 20).header().contains('…'),
                "header name truncated"
            );
        }

        /// Every list row is a game, so the row a click lands on is the game it
        /// opens. A message drawn among the rows would shift them under the mouse.
        #[test]
        fn a_reported_problem_does_not_shift_the_menu_rows() {
            let mut harness = Harness::new("1. e4 1-0 1. Ra3 0-1 1. d4 1/2-1/2");
            harness.open(1); // unplayable -> sets the status
            assert!(harness.status().is_some());

            let screen = harness.render(80, 20);
            let first_row = screen.text().lines().nth(1).unwrap_or_default().to_string();
            assert!(
                first_row.contains("1. Game 1"),
                "the first row must still be the first game, got {first_row:?}"
            );

            harness.click_menu_row(0);
            assert_eq!(
                harness.current_game(),
                0,
                "clicking the first row opens the first game"
            );
        }
    }

    mod geometry {
        use super::*;

        /// A board square must keep the same shape at every terminal size. Its
        /// pixel width and height should match, which with the default 2:1 cell
        /// aspect means the square is always twice as many columns as rows.
        #[test]
        fn squares_stay_square_at_every_terminal_size() {
            for (width, height) in [
                (80u16, 24u16),
                (100, 30),
                (120, 40),
                (140, 50),
                (90, 44),
                (200, 50),
                (160, 60),
                (110, 28),
                (70, 20),
            ] {
                let screen = Harness::new(SAMPLE).render(width, height);
                let board = screen
                    .board()
                    .unwrap_or_else(|| panic!("no board drawn at {width}x{height}"));
                assert_eq!(
                    board.cell_width,
                    board.cell_height * 2,
                    "square skewed at {width}x{height}: cell is {}x{}",
                    board.cell_width,
                    board.cell_height
                );
            }
        }

        /// Pieces must sit in the middle of their square at every board size.
        #[test]
        fn pieces_are_centered_on_their_squares() {
            for (width, height) in [(100u16, 34u16), (120, 40), (140, 48), (160, 56), (110, 36)] {
                let screen = Harness::new("1. e4 e5").render(width, height);
                let board = screen
                    .board()
                    .unwrap_or_else(|| panic!("no board at {width}x{height}"));
                // Below this the square is too small for a sprite and a single
                // glyph is drawn instead, which cannot be centered in an
                // even-width cell.
                if board.cell_width < 6 || board.cell_height < 3 {
                    continue;
                }
                for file in 0..8u16 {
                    for rank in 0..8u16 {
                        let Some(ink) = board.ink(file, rank) else {
                            continue; // empty square
                        };
                        assert_eq!(
                            ink.leading, ink.trailing,
                            "piece off-center horizontally at {width}x{height}"
                        );
                        assert_eq!(
                            ink.above, ink.below,
                            "piece off-center vertically at {width}x{height}"
                        );
                    }
                }
            }
        }

        /// The sprite is scaled to the square, so a bigger board draws a wider
        /// piece. Measures one piece's ink, because a count across the whole
        /// board grows with the board anyway and would pass with a fixed sprite.
        #[test]
        fn pieces_grow_with_the_board() {
            fn rook_ink_width(width: u16, height: u16) -> u16 {
                let screen = Harness::new("1. e4").render(width, height);
                let board = screen
                    .board()
                    .unwrap_or_else(|| panic!("no board at {width}x{height}"));
                board
                    .ink(0, 0) // a1 is the bottom-left square
                    .unwrap_or_else(|| panic!("no rook drawn at {width}x{height}"))
                    .width
            }

            let small = rook_ink_width(100, 34); // 6x3 squares
            let large = rook_ink_width(160, 56); // 12x6 squares
            assert!(
                large > small,
                "a larger board should draw a wider piece: {small} -> {large}"
            );
        }
    }

    mod layout_invariants {
        use super::*;

        /// Nothing may draw outside its panel: every rendered row must be exactly
        /// the width of the terminal.
        #[test]
        fn no_row_ever_spills_past_the_terminal() {
            let long = "[White \"Aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"] \
                        [Black \"Bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\"] 1. e4 e5 1-0 \
                        1. d4 1-0";
            for (width, height) in [(20u16, 10u16), (40, 12), (80, 24), (120, 40)] {
                for open in [false, true] {
                    let mut harness = Harness::new(long);
                    if open {
                        harness.open(0);
                    }
                    for line in harness.render(width, height).text().lines() {
                        assert_eq!(
                            line.chars().count(),
                            width as usize,
                            "row wrong width at {width}x{height}, open={open}"
                        );
                    }
                }
            }
        }
    }

    mod mouse {
        use super::*;

        #[test]
        fn clicking_a_menu_row_opens_that_game() {
            let mut harness = Harness::new(TWO_GAMES);
            harness.click_menu_row(1); // the second row is the second game
            assert_eq!(harness.mode(), Mode::Board);
            assert_eq!(harness.current_game(), 1);
        }

        #[test]
        fn clicking_the_all_games_button_returns_to_the_menu() {
            let mut harness = Harness::new(TWO_GAMES);
            harness.open(0);
            harness.click_button(0);
            assert_eq!(harness.mode(), Mode::Menu);
        }

        #[test]
        fn non_left_mouse_events_do_nothing() {
            let mut harness = Harness::new(TWO_GAMES);
            let (x, y) = harness.menu_row_xy(0);
            harness.mouse(MouseEventKind::Down(MouseButton::Right), x, y);
            harness.mouse(MouseEventKind::Moved, x, y);
            assert_eq!(harness.mode(), Mode::Menu, "only a left click opens a game");
        }

        #[test]
        fn clicking_past_the_last_game_opens_nothing() {
            let mut harness = Harness::new(TWO_GAMES); // two games
                                                       // Row 20 is well below the list; the click must not open a game.
            let (x, y) = harness.menu_row_xy(20);
            harness.mouse(MouseEventKind::Down(MouseButton::Left), x, y);
            assert_eq!(
                harness.mode(),
                Mode::Menu,
                "a click past the list opens nothing"
            );
        }
    }

    mod event_decoding {
        use super::*;

        #[test]
        fn release_key_events_do_nothing() {
            let mut harness = Harness::new(SAMPLE);
            // A press of 'q' quits; a release of it must be ignored, so a key is
            // not acted on twice where both press and release are reported.
            harness.send_key(KeyCode::Char('q'), KeyEventKind::Release);
            assert!(!harness.should_quit(), "a release event must not act");

            // The same key as a press still works: the filter is on kind, not key.
            harness.send_key(KeyCode::Char('q'), KeyEventKind::Press);
            assert!(harness.should_quit());
        }
    }

    mod scrolling {
        use super::*;
        use std::fmt::Write;

        /// The menu must scroll so that the cursor is always on screen, however
        /// far down the list it is.
        #[test]
        fn the_menu_scrolls_to_keep_the_selection_visible() {
            let pgn = "1. e4 1-0 ".repeat(30);
            let mut harness = Harness::new(&pgn);
            assert_eq!(harness.game_count(), 30);
            assert_eq!(
                harness.menu_scroll(10),
                0,
                "no scroll while the cursor fits"
            );

            harness.select(12);
            assert_eq!(
                harness.menu_scroll(10),
                3,
                "cursor pulled onto the last row"
            );

            harness.select(29);
            let scroll = harness.menu_scroll(10);
            assert!(
                scroll <= 29 && 29 < scroll + 10,
                "the selected row must be on screen"
            );
        }

        #[test]
        fn the_selected_game_is_always_visible() {
            let pgn = "1. e4 1-0 ".repeat(40);
            let mut harness = Harness::new(&pgn);
            for selected in [0usize, 5, 20, 39] {
                harness.select(selected);
                let list = harness.render(60, 16).menu_list();
                let wanted = format!("{}. Game {}", selected + 1, selected + 1);
                assert!(
                    list.contains(&wanted),
                    "game {} not visible when selected",
                    selected + 1
                );
            }
        }

        #[test]
        fn the_move_list_scrolls_to_keep_the_current_move_visible() {
            // A long game; the move list numbers its pairs 1..30 regardless of
            // the numbers in the text, so those labels make good probes.
            let mut pgn = String::new();
            for pair in 0..15 {
                write!(pgn, "{}. Nf3 Nf6 {}. Ng1 Ng8 ", 2 * pair + 1, 2 * pair + 2).unwrap();
            }
            let mut harness = Harness::new(&pgn);
            harness.open(0);

            assert!(
                harness.render(90, 16).moves().contains("  1. "),
                "first pair shown at the start"
            );

            harness.go_to_end();
            let moves = harness.render(90, 16).moves();
            assert!(
                !moves.contains("  1. "),
                "first pair scrolled off near the end"
            );
            assert!(moves.contains(" 30. "), "last pair scrolled into view");
        }
    }

    mod command_line {
        use super::*;

        fn args(items: &[&str]) -> std::vec::IntoIter<OsString> {
            items
                .iter()
                .map(|item| OsString::from(*item))
                .collect::<Vec<_>>()
                .into_iter()
        }

        #[test]
        fn no_argument_resolves_to_no_path() {
            assert_eq!(pgn_path_argument(args(&[])).unwrap(), None);
        }

        #[test]
        fn one_argument_resolves_to_that_path() {
            assert_eq!(
                pgn_path_argument(args(&["game.pgn"])).unwrap(),
                Some(PathBuf::from("game.pgn"))
            );
        }

        #[test]
        fn more_than_one_argument_is_an_error() {
            assert!(pgn_path_argument(args(&["a.pgn", "b.pgn"])).is_err());
        }
    }

    mod empty_state {
        use super::*;

        #[test]
        fn empty_app_starts_in_the_menu_with_no_games() {
            let harness = Harness::empty("nothing here");
            assert_eq!(harness.mode(), Mode::Menu);
            assert_eq!(harness.game_count(), 0);
        }

        #[test]
        fn empty_app_shows_its_message() {
            let mut harness = Harness::empty("nothing here");
            assert!(harness
                .render(80, 24)
                .menu_bottom()
                .contains("nothing here"));
        }

        #[test]
        fn empty_app_ignores_navigation_without_opening_a_board() {
            let mut harness = Harness::empty("nothing here");
            for key in [KeyCode::Down, KeyCode::Up, KeyCode::End, KeyCode::Enter] {
                harness.press(key);
                assert_eq!(
                    harness.mode(),
                    Mode::Menu,
                    "empty menu must stay in the menu"
                );
            }
            // The message is not cleared by navigating an empty menu.
            assert!(harness
                .render(80, 24)
                .menu_bottom()
                .contains("nothing here"));
        }
    }

    mod default_directory {
        use super::*;

        /// Runs `body` with a fresh empty temporary directory, removing it after.
        fn with_temp_dir<T>(body: impl FnOnce(&Path) -> T) -> T {
            static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            let unique = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let dir = std::env::temp_dir()
                .join(format!("chessview-test-{}-{unique}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let result = body(&dir);
            let _ = std::fs::remove_dir_all(&dir);
            result
        }

        #[test]
        fn reads_the_only_file() {
            with_temp_dir(|dir| {
                std::fs::write(dir.join("game.pgn"), "1. e4 e5 1-0").unwrap();
                let text = read_single_pgn(dir).unwrap();
                assert!(text.contains("e4"));
            });
        }

        #[test]
        fn errors_on_an_empty_directory() {
            with_temp_dir(|dir| {
                assert!(read_single_pgn(dir).is_err());
            });
        }

        #[test]
        fn errors_with_more_than_one_file() {
            with_temp_dir(|dir| {
                std::fs::write(dir.join("a.pgn"), "1. e4 1-0").unwrap();
                std::fs::write(dir.join("b.pgn"), "1. d4 1-0").unwrap();
                assert!(read_single_pgn(dir).is_err());
            });
        }
    }
}
