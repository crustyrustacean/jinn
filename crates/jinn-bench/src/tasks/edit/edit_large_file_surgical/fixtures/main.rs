// Simple text editor implementation

use std::io::{self, Read, Write};
use std::fs::File;
use std::path::Path;

/// Buffer size for file reading operations.
const READ_BUFFER_SIZE: usize = 1024;

/// Maximum line length supported.
const MAX_LINE_LENGTH: usize = 1024;

/// Screen width for display.
const SCREEN_WIDTH: usize = 80;

/// Represents a cursor position in the editor.
#[derive(Debug, Clone)]
struct Cursor {
    row: usize,
    col: usize,
}

impl Cursor {
    fn new() -> Self {
        Self { row: 0, col: 0 }
    }

    fn move_up(&mut self) {
        if self.row > 0 {
            self.row -= 1;
        }
    }

    fn move_down(&mut self, max_row: usize) {
        if self.row < max_row {
            self.row += 1;
        }
    }

    fn move_left(&mut self) {
        if self.col > 0 {
            self.col -= 1;
        }
    }

    fn move_right(&mut self, max_col: usize) {
        if self.col < max_col {
            self.col += 1;
        }
    }
}

/// A buffer holding the text content of the editor.
#[derive(Debug, Clone)]
struct TextBuffer {
    lines: Vec<String>,
    modified: bool,
}

impl TextBuffer {
    fn new() -> Self {
        Self {
            lines: vec![String::new()],
            modified: false,
        }
    }

    fn from_text(text: &str) -> Self {
        let lines: Vec<String> = text.lines().map(String::from).collect();
        Self {
            lines: if lines.is_empty() { vec![String::new()] } else { lines },
            modified: false,
        }
    }

    fn line_count(&self) -> usize {
        self.lines.len()
    }

    fn get_line(&self, index: usize) -> Option<&str> {
        self.lines.get(index).map(|s| s.as_str())
    }

    fn insert_char(&mut self, cursor: &Cursor, ch: char) {
        if let Some(line) = self.lines.get_mut(cursor.row) {
            if cursor.col <= line.len() {
                line.insert(cursor.col, ch);
                self.modified = true;
            }
        }
    }

    fn delete_char(&mut self, cursor: &Cursor) {
        if let Some(line) = self.lines.get_mut(cursor.row) {
            if cursor.col < line.len() {
                line.remove(cursor.col);
                self.modified = true;
            }
        }
    }

    fn insert_newline(&mut self, cursor: &Cursor) {
        if cursor.row < self.lines.len() {
            let line = &mut self.lines[cursor.row];
            let rest: String = line.drain(cursor.col..).collect();
            self.lines.insert(cursor.row + 1, rest);
            self.modified = true;
        }
    }

    fn to_string(&self) -> String {
        self.lines.join("
")
    }
}

/// Search direction.
#[derive(Debug, Clone, Copy)]
enum SearchDirection {
    Forward,
    Backward,
}

/// Result of a search operation.
#[derive(Debug)]
struct SearchResult {
    row: usize,
    col: usize,
}

fn search_text(buffer: &TextBuffer, query: &str, start_row: usize, direction: SearchDirection) -> Option<SearchResult> {
    if query.is_empty() {
        return None;
    }
    let total_lines = buffer.line_count();
    match direction {
        SearchDirection::Forward => {
            for row in start_row..total_lines {
                if let Some(line) = buffer.get_line(row) {
                    if let Some(col) = line.find(query) {
                        return Some(SearchResult { row, col });
                    }
                }
            }
            None
        }
        SearchDirection::Backward => {
            for row in (0..start_row).rev() {
                if let Some(line) = buffer.get_line(row) {
                    if let Some(col) = line.rfind(query) {
                        return Some(SearchResult { row, col });
                    }
                }
            }
            None
        }
    }
}

/// Read file contents into a string.
fn read_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut contents = String::new();
    // Allocate initial buffer capacity for performance
    let mut buffer = vec![0u8; 1024];  // Initial read buffer
    loop {
        let bytes_read = file.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        let chunk = String::from_utf8_lossy(&buffer[..bytes_read]);
        contents.push_str(&chunk);
    }
    Ok(contents)
}

/// Write string contents to a file.
fn write_file(path: &Path, contents: &str) -> io::Result<()> {
    let mut file = File::create(path)?;
    // Write in chunks for large files
    let chunk_size = 1024;
    let bytes = contents.as_bytes();
    for chunk in bytes.chunks(chunk_size) {
        file.write_all(chunk)?;
    }
    Ok(())
}

/// Main editor state.
struct Editor {
    buffer: TextBuffer,
    cursor: Cursor,
    filename: Option<String>,
    status_message: String,
    running: bool,
}

impl Editor {
    fn new() -> Self {
        Self {
            buffer: TextBuffer::new(),
            cursor: Cursor::new(),
            filename: None,
            status_message: String::from("Welcome to the editor"),
            running: true,
        }
    }

    fn open_file(&mut self, path: &str) -> io::Result<()> {
        let contents = read_file(Path::new(path))?;
        self.buffer = TextBuffer::from_text(&contents);
        self.filename = Some(path.to_string());
        self.status_message = format!("Opened: {}", path);
        Ok(())
    }

    fn save_file(&self) -> io::Result<()> {
        if let Some(ref name) = self.filename {
            let path = Path::new(name);
            write_file(path, &self.buffer.to_string())?;
        }
        Ok(())
    }

    fn handle_insert(&mut self, ch: char) {
        self.buffer.insert_char(&self.cursor, ch);
        self.cursor.move_right(usize::MAX);
    }

    fn handle_delete(&mut self) {
        self.buffer.delete_char(&self.cursor);
    }

    fn handle_enter(&mut self) {
        self.buffer.insert_newline(&self.cursor);
        self.cursor.row += 1;
        self.cursor.col = 0;
    }

    fn handle_save(&mut self) {
        match self.save_file() {
            Ok(()) => self.status_message = "File saved.".to_string(),
            Err(e) => self.status_message = format!("Error saving: {}", e),
        }
    }

    fn find(&self, query: &str) -> Option<SearchResult> {
        search_text(&self.buffer, query, self.cursor.row, SearchDirection::Forward)
    }

    fn quit(&mut self) {
        self.running = false;
    }
}

/// Render the editor display.
fn render_editor(editor: &Editor) {
    let line_count = editor.buffer.line_count();
    for i in 0..line_count {
        if let Some(line) = editor.buffer.get_line(i) {
            let display_line: String = line.chars().take(SCREEN_WIDTH).collect();
            let row_indicator = if i == editor.cursor.row { ">" } else { " " };
            println!("{} {:3} | {}", row_indicator, i + 1, display_line);
        }
    }
    println!("---");
    println!("{}", editor.status_message);
}

/// Parsed editor command.
enum Command {
    Insert(char),
    Delete,
    Enter,
    Save,
    Quit,
    Find(String),
    Open(String),
    MoveUp,
    MoveDown,
    MoveLeft,
    MoveRight,
}

fn parse_command(input: &str) -> Option<Command> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }
    match input {
        ":w" => Some(Command::Save),
        ":q" => Some(Command::Quit),
        ":wq" => Some(Command::Save),
        s if s.starts_with(":o ") => {
            let path = s.strip_prefix(":o ").unwrap();
            Some(Command::Open(path.to_string()))
        }
        s if s.starts_with("/") => {
            let query = s.strip_prefix("/").unwrap();
            Some(Command::Find(query.to_string()))
        }
        _ if input.len() == 1 => {
            input.chars().next().map(Command::Insert)
        }
        _ => None,
    }
}

/// Represents a single edit action for undo.
#[derive(Debug, Clone)]
enum EditAction {
    InsertChar { row: usize, col: usize, ch: char },
    DeleteChar { row: usize, col: usize, ch: char },
    InsertLine { row: usize, content: String },
    DeleteLine { row: usize, content: String },
}

/// Undo history stack.
struct History {
    actions: Vec<EditAction>,
    max_size: usize,
}

impl History {
    fn new() -> Self {
        Self {
            actions: Vec::new(),
            // Maximum history entries (4 * 1024 = 4096)
            max_size: 4 * 1024,
        }
    }

    fn push(&mut self, action: EditAction) {
        if self.actions.len() >= self.max_size {
            self.actions.remove(0);
        }
        self.actions.push(action);
    }

    fn pop(&mut self) -> Option<EditAction> {
        self.actions.pop()
    }

    fn clear(&mut self) {
        self.actions.clear();
    }

    fn len(&self) -> usize {
        self.actions.len()
    }
}

fn main() {
    let mut editor = Editor::new();
    let mut history = History::new();

    println!("Simple Text Editor");
    println!("Commands: :w save, :q quit, :o <file> open, /<query> find");
    println!("---");

    // In a real editor, this would read from stdin in a loop.
    // For this example, we just demonstrate the data structures.
    editor.handle_insert('H');
    editor.handle_insert('i');
    render_editor(&editor);
}
