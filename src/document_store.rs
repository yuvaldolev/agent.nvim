use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use lsp_types::{Position, Url};

#[derive(Debug, Clone)]
pub struct Document {
    pub text: String,
    pub version: i32,
    pub language_id: String,
}

#[derive(Debug, Clone)]
pub struct DocumentStore {
    documents: Arc<Mutex<HashMap<Url, Document>>>,
}

impl DocumentStore {
    pub fn new() -> Self {
        Self {
            documents: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn open(&self, uri: Url, text: String, version: i32, language_id: String) {
        let mut docs = self.documents.lock().unwrap();
        docs.insert(
            uri,
            Document {
                text,
                version,
                language_id,
            },
        );
    }

    pub fn change(
        &self,
        uri: &Url,
        version: i32,
        changes: &[lsp_types::TextDocumentContentChangeEvent],
    ) {
        let mut docs = self.documents.lock().unwrap();
        if let Some(doc) = docs.get_mut(uri) {
            doc.version = version;
            for change in changes {
                if let Some(range) = change.range {
                    let start_offset = position_to_offset(&doc.text, range.start);
                    let end_offset = position_to_offset(&doc.text, range.end);
                    doc.text
                        .replace_range(start_offset..end_offset, &change.text);
                } else {
                    doc.text = change.text.clone();
                }
            }
        }
    }

    pub fn get(&self, uri: &Url) -> Option<Document> {
        let docs = self.documents.lock().unwrap();
        docs.get(uri).cloned()
    }
}

impl Default for DocumentStore {
    fn default() -> Self {
        Self::new()
    }
}

fn position_to_offset(text: &str, position: Position) -> usize {
    let mut offset = 0;
    for (line_num, line) in text.lines().enumerate() {
        if line_num == position.line as usize {
            return offset + position.character as usize;
        }
        offset += line.len() + 1;
    }
    offset
}

#[allow(dead_code)]
fn offset_to_position(text: &str, offset: usize) -> Position {
    let mut current_offset = 0;
    for (line_num, line) in text.lines().enumerate() {
        let line_end = current_offset + line.len() + 1;
        if offset < line_end {
            return Position {
                line: line_num as u32,
                character: (offset - current_offset) as u32,
            };
        }
        current_offset = line_end;
    }
    Position {
        line: text.lines().count() as u32,
        character: 0,
    }
}
