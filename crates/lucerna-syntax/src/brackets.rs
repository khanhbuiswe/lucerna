use std::ops::Range;

use lucerna_text::{BufferSnapshot, Point};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Node, Query, QueryCursor, Tree};

/// How far the text scan looks for a partner before giving up.
const SCAN_LIMIT: usize = 1 << 20;

const PAIRS: [(u8, u8); 3] = [(b'(', b')'), (b'[', b']'), (b'{', b'}')];

/// The offset of the bracket paired with the one at `offset` (or just before
/// it), found by scanning the text with a depth counter. Use
/// [`crate::SyntaxState::matching_bracket`] when a tree exists, so brackets
/// in strings and comments are ignored.
pub fn matching_bracket(snapshot: &BufferSnapshot, offset: usize) -> Option<usize> {
    let (pos, bracket) = [Some(offset), offset.checked_sub(1)]
        .into_iter()
        .flatten()
        .find_map(|o| {
            byte_at(snapshot, o)
                .filter(|&b| pair(b).is_some())
                .map(|b| (o, b))
        })?;
    let (open, close) = pair(bracket)?;
    let len = snapshot.len_bytes();
    if bracket == open {
        let mut depth = 0usize;
        let window = pos + 1..len.min(pos + 1 + SCAN_LIMIT);
        for (at, byte) in bytes_in(snapshot, window) {
            if byte == open {
                depth += 1;
            } else if byte == close {
                if depth == 0 {
                    return Some(at);
                }
                depth -= 1;
            }
        }
        None
    } else {
        // Scan forward up to `pos`; the innermost still-open bracket pairs.
        let mut opens = Vec::new();
        for (at, byte) in bytes_in(snapshot, pos.saturating_sub(SCAN_LIMIT)..pos) {
            if byte == open {
                opens.push(at);
            } else if byte == close {
                opens.pop();
            }
        }
        opens.pop()
    }
}

/// The bracket paired with the leaf bracket at `offset` (or just before it),
/// found among the leaf's siblings. `None` when neither is a bracket leaf.
pub(crate) fn matching_bracket_in_tree(tree: &Tree, offset: usize) -> Option<usize> {
    let leaf = bracket_leaf(tree, offset)
        .or_else(|| offset.checked_sub(1).and_then(|o| bracket_leaf(tree, o)))?;
    let bracket = leaf.kind().as_bytes()[0];
    let (open, close) = pair(bracket)?;
    let partner = if bracket == open { close } else { open };
    let parent = leaf.parent()?;
    let mut cursor = parent.walk();
    let siblings: Vec<Node> = parent.children(&mut cursor).collect();
    let index = siblings.iter().position(|n| n.id() == leaf.id())?;
    let candidates: Box<dyn Iterator<Item = &Node>> = if bracket == open {
        Box::new(siblings[index + 1..].iter())
    } else {
        Box::new(siblings[..index].iter().rev())
    };
    let mut depth = 0usize;
    for node in candidates.filter(|n| n.child_count() == 0) {
        let kind = node.kind().as_bytes();
        if kind == [bracket] {
            depth += 1;
        } else if kind == [partner] {
            if depth == 0 {
                return Some(node.start_byte());
            }
            depth -= 1;
        }
    }
    None
}

/// The bracket paired with the one at `offset` (or just before it), by the
/// language's `brackets.scm`: each match holds an `@open` and a `@close`.
pub(crate) fn matching_bracket_by_query(
    tree: &Tree,
    query: &Query,
    snapshot: &BufferSnapshot,
    offset: usize,
) -> Option<usize> {
    let open = query.capture_index_for_name("open")?;
    let close = query.capture_index_for_name("close")?;
    let len = snapshot.len_bytes();
    let text = move |node: Node| {
        let range = node.byte_range();
        snapshot
            .chunks_in_range(range.start.min(len)..range.end.min(len))
            .map(str::as_bytes)
    };
    for at in [Some(offset), offset.checked_sub(1)].into_iter().flatten() {
        let mut cursor = QueryCursor::new();
        cursor.set_byte_range(at..at + 1);
        let mut matches = cursor.matches(query, tree.root_node(), text);
        while let Some(m) = matches.next() {
            let node = |index| m.nodes_for_capture_index(index).next();
            let (Some(o), Some(c)) = (node(open), node(close)) else {
                continue;
            };
            if o.start_byte() == at {
                return Some(c.start_byte());
            }
            if c.start_byte() == at {
                return Some(o.start_byte());
            }
        }
    }
    None
}

fn bracket_leaf(tree: &Tree, start: usize) -> Option<Node<'_>> {
    let node = tree
        .root_node()
        .descendant_for_byte_range(start, start + 1)?;
    let is_bracket = matches!(node.kind().as_bytes(), [b] if pair(*b).is_some());
    (is_bracket
        && node.child_count() == 0
        && node.start_byte() == start
        && node.end_byte() == start + 1)
        .then_some(node)
}

fn pair(byte: u8) -> Option<(u8, u8)> {
    PAIRS.into_iter().find(|&(o, c)| byte == o || byte == c)
}

fn byte_at(snapshot: &BufferSnapshot, offset: usize) -> Option<u8> {
    bytes_in(snapshot, offset..offset + 1)
        .next()
        .map(|(_, b)| b)
}

/// The bytes of `window` with their offsets. Slices only at line starts, so
/// `window` need not fall on char boundaries.
fn bytes_in(
    snapshot: &BufferSnapshot,
    window: Range<usize>,
) -> impl Iterator<Item = (usize, u8)> + '_ {
    let window = window.start..window.end.min(snapshot.len_bytes());
    let from = line_start(snapshot, window.start);
    let to = line_end(snapshot, window.end);
    snapshot
        .chunks_in_range(from..to)
        .scan(from, |pos, chunk| {
            let start = *pos;
            *pos += chunk.len();
            Some((start, chunk))
        })
        .skip_while(move |(start, chunk)| start + chunk.len() <= window.start)
        .take_while(move |(start, _)| *start < window.end)
        .flat_map(|(start, chunk)| chunk.bytes().enumerate().map(move |(i, b)| (start + i, b)))
        .filter(move |(at, _)| window.contains(at))
}

fn line_start(snapshot: &BufferSnapshot, offset: usize) -> usize {
    let row = snapshot.offset_to_point(offset).row;
    snapshot.point_to_offset(Point { row, column: 0 })
}

fn line_end(snapshot: &BufferSnapshot, offset: usize) -> usize {
    crate::row_start(snapshot, snapshot.offset_to_point(offset).row + 1)
}
