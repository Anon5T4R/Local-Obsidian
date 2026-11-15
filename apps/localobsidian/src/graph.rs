//! The link graph: the vault's notes as nodes, its `[[links]]` as edges.
//!
//! Not a physics simulation. Notes sit on rings - one ring per folder, notes alphabetical inside it,
//! the busiest note in the middle - which makes the same vault draw the same way every time and
//! stays readable with a few hundred notes. Hovering a note lights up its links; clicking opens it.
//!
//! Port of the upstream `GraphView` in what it shows (notes, links, click to open), not in how it
//! moves: a force layout that never settles is pretty for a screenshot and unusable for reading.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::tl;
use crate::vault_state::VaultState;

/// Most notes the graph draws. Past this the rings turn into a hairball; the busiest notes win.
const MAX_NODES: usize = 240;

/// A note in the graph.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub path: PathBuf,
    pub name: String,
    /// Unit coordinates (0..1); the painter scales them to the canvas.
    pub at: (f32, f32),
    /// How many links touch it.
    pub degree: usize,
    /// The busiest note, drawn in the middle.
    pub is_hub: bool,
}

/// A link between two nodes, by index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edge {
    pub from: usize,
    pub to: usize,
}

/// Lays the vault out: `(nodes, edges)`.
pub fn layout(state: &VaultState) -> (Vec<Node>, Vec<Edge>) {
    // Which notes are worth drawing when the vault is too big for one screen.
    let mut degree: BTreeMap<PathBuf, usize> = BTreeMap::new();
    for note in &state.files {
        degree.insert(note.path.clone(), 0);
    }
    let mut links: Vec<(PathBuf, PathBuf)> = Vec::new();
    for note in &state.files {
        let Some(content) = state.contents_of(&note.path) else { continue };
        for target in localobsidian_md::links::extract(content) {
            // `[[Protocolo#Choque]]` links to the note `Protocolo`: the section is not part of the name.
            let name = localobsidian_md::links::parse_target(&target).note;
            let Some(other) = state.resolve_note_from(name, Some(note.path.as_path())) else { continue };
            if other.path == note.path {
                continue;
            }
            *degree.entry(note.path.clone()).or_default() += 1;
            *degree.entry(other.path.clone()).or_default() += 1;
            links.push((note.path.clone(), other.path.clone()));
        }
    }

    let mut files: Vec<&localobsidian_vault::NoteFile> = state.files.iter().collect();
    files.sort_by(|a, b| {
        let folder = |note: &localobsidian_vault::NoteFile| note.relative_path.split('/').next().unwrap_or_default().to_string();
        folder(a).cmp(&folder(b)).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    if files.len() > MAX_NODES {
        let mut ranked = files.clone();
        ranked.sort_by_key(|note| std::cmp::Reverse(degree.get(&note.path).copied().unwrap_or(0)));
        ranked.truncate(MAX_NODES);
        files = ranked;
    }

    // The busiest note takes the centre and the rest sit on a ring: links then radiate instead of
    // crossing the whole canvas, and the same vault always draws the same picture.
    let hub = files
        .iter()
        .enumerate()
        .filter(|(_, note)| degree.get(&note.path).copied().unwrap_or(0) >= 3)
        .max_by_key(|(index, note)| (degree.get(&note.path).copied().unwrap_or(0), std::cmp::Reverse(*index)))
        .map(|(index, _)| index)
        .filter(|_| files.len() > 3);
    let ring: Vec<usize> = (0..files.len()).filter(|index| Some(*index) != hub).collect();
    let ring_count = ring.len().max(1);
    // A crowded ring gets a wider one, so the labels keep some air.
    let radius = if ring_count > 40 { 0.46_f32 } else { 0.42 };

    let mut positions = vec![(0.5_f32, 0.5_f32); files.len()];
    for (position, &file_index) in ring.iter().enumerate() {
        let angle = std::f32::consts::TAU * (position as f32) / (ring_count as f32) - std::f32::consts::FRAC_PI_2;
        positions[file_index] = (0.5 + radius * angle.cos(), 0.5 + radius * angle.sin());
    }
    if let Some(hub_index) = hub {
        positions[hub_index] = (0.5, 0.5);
    }

    let nodes: Vec<Node> = files
        .iter()
        .enumerate()
        .map(|(index, note)| Node {
            path: note.path.clone(),
            name: note.name.clone(),
            at: positions[index],
            degree: degree.get(&note.path).copied().unwrap_or(0),
            is_hub: Some(index) == hub,
        })
        .collect();
    let index_of: BTreeMap<&PathBuf, usize> = nodes.iter().enumerate().map(|(index, node)| (&node.path, index)).collect();
    let edges: Vec<Edge> = links
        .iter()
        .filter_map(|(from, to)| {
            let (&a, &b) = (index_of.get(from)?, index_of.get(to)?);
            (a != b).then_some(Edge { from: a, to: b })
        })
        .collect();
    (nodes, edges)
}

/// How many notes still get a permanent label; past this the labels would overlap.
const LABELLED_MAX: usize = 64;

/// How close the pointer has to be to a node to count as over it. A node is at most 6 px across;
/// the extra room is what makes a graph of small dots clickable.
const HIT_RADIUS: f32 = 9.0;

/// Where a node sits inside the canvas.
fn screen_at(rect: egui::Rect, node: &Node) -> egui::Pos2 {
    egui::pos2(rect.left() + node.at.0 * rect.width(), rect.top() + node.at.1 * rect.height())
}

/// The node under `pointer`, or `None` when the pointer is in the empty space between nodes.
pub fn node_at(nodes: &[Node], rect: egui::Rect, pointer: egui::Pos2) -> Option<usize> {
    nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (index, screen_at(rect, node).distance(pointer)))
        .filter(|(_, distance)| *distance < HIT_RADIUS)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(index, _)| index)
}

/// Draws the graph. Returns the note the user clicked.
pub fn show(ui: &mut egui::Ui, state: &VaultState) -> Option<PathBuf> {
    let (nodes, edges) = layout(state);
    if nodes.is_empty() {
        ui.vertical_centered(|ui| {
            ui.add_space(24.0);
            ui.label(egui::RichText::new(tl!("there are no notes to draw")).weak());
        });
        return None;
    }

    let mut opened = None;
    let size = ui.available_size();
    let (response, painter) = ui.allocate_painter(size, egui::Sense::click());
    // The drawing keeps a margin, so a label on a note at the edge has somewhere to go.
    let canvas = response.rect.shrink2(egui::vec2(30.0, 20.0));

    let hovered = response.hover_pos().and_then(|pointer| node_at(&nodes, canvas, pointer));

    // Every note is named while the graph is small enough to read; past that only the busiest get a
    // permanent label and the rest introduce themselves on hover.
    let labelled = nodes.len() <= LABELLED_MAX;
    let text = ui.visuals().strong_text_color();
    let faint = ui.visuals().weak_text_color();
    let link = faint.gamma_multiply(0.9);
    let bright = ui.visuals().selection.bg_fill;
    for edge in &edges {
        let lit = hovered.is_some_and(|index| index == edge.from || index == edge.to);
        painter.line_segment(
            [screen_at(canvas, &nodes[edge.from]), screen_at(canvas, &nodes[edge.to])],
            egui::Stroke::new(if lit { 1.8 } else { 1.0 }, if lit { bright } else { link }),
        );
    }
    for (index, node) in nodes.iter().enumerate() {
        let centre = screen_at(canvas, node);
        let lit = hovered == Some(index);
        let radius = if lit { 7.0 } else { 4.5 + f32::min(node.degree as f32, 5.0) * 0.6 };
        painter.circle_filled(centre, radius, if lit { bright } else { faint.gamma_multiply(1.4) });
        // The label sits on the far side of the ring, so it never covers the links that leave the
        // centre; the hub and the hovered note are labelled towards the open space instead.
        let outward = egui::vec2(centre.x - canvas.center().x, centre.y - canvas.center().y);
        let direction = if outward.length() < 1.0 { egui::vec2(0.0, -1.0) } else { outward.normalized() };
        if lit || labelled || node.is_hub {
            let font = egui::FontId::proportional(if lit { 13.5 } else { 12.0 });
            let colour = if lit { text } else { faint };
            // Keep the label inside the canvas: a note at the edge would otherwise write its name
            // over its own dot, or off the picture.
            let half = painter.layout_no_wrap(node.name.clone(), font.clone(), colour).size().x / 2.0;
            // Sideways the label needs its own width of room, otherwise the name starts on top of
            // the dot; above and below, the dot's own radius is enough.
            let distance = if direction.x.abs() > 0.5 { half + radius + 6.0 } else { radius + 8.0 };
            let mut anchor = centre + direction * distance;
            anchor.x = anchor.x.clamp(canvas.left() + half + 4.0, canvas.right() - half - 4.0);
            anchor.y = anchor.y.clamp(canvas.top() + 8.0, canvas.bottom() - 8.0);
            painter.text(anchor, egui::Align2::CENTER_CENTER, &node.name, font, colour);
        }
    }

    // The active note is ringed: it is the one the user came from.
    if let Some(active) = &state.active
        && let Some(node) = nodes.iter().find(|node| &node.path == active)
    {
        painter.circle_stroke(screen_at(canvas, node), 8.0, egui::Stroke::new(1.5, ui.visuals().warn_fg_color));
        // A crowded graph has no permanent labels, so the current note names itself.
        if !labelled {
            painter.text(screen_at(canvas, node) + egui::vec2(10.0, 6.0), egui::Align2::LEFT_TOP, &node.name, egui::FontId::proportional(13.0), text);
        }
    }

    if response.clicked()
        && let Some(index) = hovered
    {
        opened = Some(nodes[index].path.clone());
    }
    if hovered.is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    opened
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn vault(notes: &[(&str, &str)]) -> VaultState {
        let mut state = VaultState::default();
        state.files = notes
            .iter()
            .map(|(relative, _)| localobsidian_vault::NoteFile {
                name: relative.rsplit('/').next().unwrap_or(relative).trim_end_matches(".md").to_string(),
                path: PathBuf::from(format!("/v/{relative}")),
                relative_path: (*relative).to_string(),
                mtime_ms: 0,
            })
            .collect();
        let contents: BTreeMap<PathBuf, String> =
            notes.iter().map(|(relative, text)| (PathBuf::from(format!("/v/{relative}")), (*text).to_string())).collect();
        state.index = localobsidian_vault::index::build(&state.files, &contents);
        state.replace_contents_for_tests(contents);
        state
    }

    #[test]
    fn every_note_is_a_node_and_every_link_an_edge() {
        let state = vault(&[("A.md", "[[B]]"), ("B.md", "[[C]]"), ("C.md", "nada")]);
        let (nodes, edges) = layout(&state);
        assert_eq!(nodes.len(), 3);
        assert_eq!(edges.len(), 2);
    }

    #[test]
    fn the_layout_is_deterministic() {
        let state = vault(&[("A.md", "[[B]]"), ("B.md", "[[A]]")]);
        let first = layout(&state);
        let second = layout(&state);
        assert_eq!(first, second, "the same vault draws the same way");
    }

    #[test]
    fn the_busiest_notes_sit_closer_to_the_centre() {
        let state = vault(&[("A.md", "[[B]] [[C]] [[D]]"), ("B.md", "nada"), ("C.md", "nada"), ("D.md", "nada")]);
        let (nodes, _) = layout(&state);
        let distance = |name: &str| {
            let node = nodes.iter().find(|node| node.name == name).expect("a node");
            ((node.at.0 - 0.5).powi(2) + (node.at.1 - 0.5).powi(2)).sqrt()
        };
        assert!(distance("A") < distance("B"), "A has three links, B none");
    }

    #[test]
    fn a_pointer_near_a_node_hits_it_and_the_empty_space_hits_nothing() {
        let state = vault(&[("A.md", "[[B]]"), ("B.md", "[[A]]")]);
        let (nodes, _) = layout(&state);
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(400.0, 400.0));
        let centre = screen_at(rect, &nodes[0]);
        assert_eq!(node_at(&nodes, rect, centre + egui::vec2(3.0, 0.0)), Some(0), "a click lands on a node");
        assert_eq!(node_at(&nodes, rect, egui::pos2(200.0, 200.0)), None, "the middle is empty");
    }

    #[test]
    fn a_link_to_a_section_is_an_edge_to_the_note() {
        let state = vault(&[("A.md", "[[B#Choque]]"), ("B.md", "nada")]);
        let (_, edges) = layout(&state);
        assert_eq!(edges.len(), 1, "the section is not part of the note's name");
    }

    #[test]
    fn a_dangling_link_is_not_an_edge() {
        let state = vault(&[("A.md", "[[Fantasma]]")]);
        let (_, edges) = layout(&state);
        assert!(edges.is_empty());
    }
}
