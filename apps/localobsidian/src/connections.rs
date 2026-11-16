//! The connection map: a note in the middle, its links around it, expanded one click at a time.
//!
//! Not the vault-wide graph. A graph of 500 notes is a hairball: pretty in a screenshot, useless to
//! read, and expensive to draw. What a reader wants is the piece of the vault around the note in
//! front of them — so the map starts at one note, shows its direct connections (links out and links
//! in), and grows where the reader clicks. No physics, no animation: rings by hop, angles by the
//! weight of each branch, so the same map draws the same way every time.
//!
//! Interaction: click a note to open it, click the badge on a note to open (or close) its own
//! connections, right-click for the menu, and the toolbar to start over from the open note.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

use crate::tl;
use crate::vault_state::VaultState;

/// Most notes the map draws at once. Past this the rings crowd and the reader is not reading.
const MAX_NODES: usize = 160;

/// The radius of the outermost ring, as a share of the canvas.
const OUTER: f32 = 0.42;

/// What the user asked the map for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    None,
    /// Open the note (navigation is what the map is for).
    Open(PathBuf),
    /// Start the map over from this note.
    Start(PathBuf),
}

/// The map's own state: where it started, and which notes have their connections open.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Map {
    origin: Option<PathBuf>,
    expanded: BTreeSet<PathBuf>,
}

impl Map {
    pub fn origin(&self) -> Option<&Path> {
        self.origin.as_deref()
    }

    /// Starts over: this note in the middle, nothing expanded.
    pub fn start_from(&mut self, path: &Path) {
        self.origin = Some(path.to_path_buf());
        self.expanded.clear();
    }

    /// Opens or closes the connections of `note`. The origin is always open.
    pub fn toggle(&mut self, note: &Path) {
        if self.origin.as_deref() == Some(note) {
            return;
        }
        if !self.expanded.remove(note) {
            self.expanded.insert(note.to_path_buf());
        }
    }

    /// Closes every expansion: back to the note and its direct connections.
    pub fn collapse_all(&mut self) {
        self.expanded.clear();
    }

    pub fn is_expanded(&self, note: &Path) -> bool {
        self.origin.as_deref() == Some(note) || self.expanded.contains(note)
    }

    /// Gives the map somewhere to start when it has never been opened for this vault.
    pub fn ensure_origin(&mut self, active: Option<&Path>) {
        if self.origin.is_none() {
            self.origin = active.map(Path::to_path_buf);
        }
    }
}

/// A note as the map draws it.
#[derive(Debug, Clone, PartialEq)]
pub struct Placed {
    pub path: PathBuf,
    pub name: String,
    /// Unit coordinates: the painter scales them to the canvas.
    pub at: (f32, f32),
    /// How many clicks away from the origin (0 is the origin itself).
    pub level: usize,
    pub expanded: bool,
    /// Whether it still has connections the map has not drawn.
    pub has_more: bool,
    /// The note this one hangs from (`None` for the origin), for the edges.
    pub parent: Option<PathBuf>,
}

/// The notes next to `path`: links out and links in, without itself.
fn neighbours(state: &VaultState, path: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    for list in [state.index.links.get(path), state.index.linked_from.get(path)].into_iter().flatten() {
        for other in list {
            if other.as_path() != path && !found.iter().any(|existing| existing == other) {
                found.push(other.clone());
            }
        }
    }
    // Ordered by the name the reader sees, so the rings are stable from frame to frame.
    let name = |path: &PathBuf| state.files.iter().find(|note| &note.path == path).map(|note| note.name.to_lowercase()).unwrap_or_default();
    found.sort_by(|a, b| name(a).cmp(&name(b)).then_with(|| a.cmp(b)));
    found
}

fn name_of(state: &VaultState, path: &Path) -> String {
    state.files.iter().find(|note| note.path == path).map(|note| note.name.clone()).unwrap_or_default()
}

/// Lays the map out: the origin in the middle and one ring per hop.
///
/// Radial, deterministic, no simulation: each branch gets an angular slice in proportion to how
/// many notes hang from it, so nothing overlaps and the same vault, origin and expansions always
/// draw the same picture (which is also what makes it testable).
pub fn layout(state: &VaultState, map: &Map) -> Vec<Placed> {
    let Some(origin) = map.origin.clone() else { return Vec::new() };
    if !state.files.iter().any(|note| note.path == origin) {
        return Vec::new();
    }
    let known: BTreeSet<PathBuf> = state.files.iter().map(|note| note.path.clone()).collect();

    // The tree of visible notes, breadth first from the origin; a note already placed stays where
    // its first path put it.
    let mut level: BTreeMap<PathBuf, usize> = BTreeMap::from([(origin.clone(), 0)]);
    let mut children: BTreeMap<PathBuf, Vec<PathBuf>> = BTreeMap::new();
    let mut parent: BTreeMap<PathBuf, PathBuf> = BTreeMap::new();
    let mut queue: VecDeque<PathBuf> = VecDeque::from([origin.clone()]);
    while let Some(current) = queue.pop_front() {
        if !map.is_expanded(&current) {
            continue;
        }
        let depth = level[&current];
        for next in neighbours(state, &current) {
            if level.len() >= MAX_NODES {
                break;
            }
            if !known.contains(&next) || level.contains_key(&next) {
                continue;
            }
            level.insert(next.clone(), depth + 1);
            parent.insert(next.clone(), current.clone());
            children.entry(current.clone()).or_default().push(next.clone());
            queue.push_back(next);
        }
    }

    // How much of the circle each branch deserves: a note with nothing under it takes one share,
    // and every parent takes the sum of its children.
    let mut weight: BTreeMap<PathBuf, f32> = BTreeMap::new();
    let mut order: Vec<PathBuf> = level.keys().cloned().collect();
    order.sort_by_key(|path| std::cmp::Reverse(level[path]));
    for path in &order {
        let below: f32 = children.get(path).map(|list| list.iter().map(|child| weight.get(child).copied().unwrap_or(1.0)).sum()).unwrap_or(0.0);
        weight.insert(path.clone(), if below > 0.0 { below } else { 1.0 });
    }

    // The slices, in a deterministic walk: the origin owns the whole circle, each child takes the
    // part of its parent's slice it earned, and the note sits in the middle of its own slice.
    let mut placed: Vec<Placed> = Vec::new();
    let mut slices: Vec<(PathBuf, f32, f32)> = vec![(origin.clone(), 0.0, std::f32::consts::TAU)];
    while let Some((path, start, end)) = slices.pop() {
        let depth = level[&path];
        let at = if depth == 0 {
            (0.5, 0.5)
        } else {
            let radius = OUTER * depth as f32 / level.values().copied().max().unwrap_or(1).max(1) as f32;
            let angle = (start + end) / 2.0 - std::f32::consts::FRAC_PI_2;
            (0.5 + radius * angle.cos(), 0.5 + radius * angle.sin())
        };
        let open = map.is_expanded(&path);
        let below = children.get(&path).cloned().unwrap_or_default();
        placed.push(Placed {
            path: path.clone(),
            name: name_of(state, &path),
            at,
            level: depth,
            expanded: open && !below.is_empty(),
            // A closed note with neighbours is one click away from growing.
            has_more: !open && neighbours(state, &path).iter().any(|next| !level.contains_key(next)),
            parent: parent.get(&path).cloned(),
        });
        let total: f32 = below.iter().map(|child| weight.get(child).copied().unwrap_or(1.0)).sum();
        let mut cursor = start;
        for child in below {
            let share = if total > 0.0 { (end - start) * weight.get(&child).copied().unwrap_or(1.0) / total } else { 0.0 };
            slices.push((child, cursor, cursor + share));
            cursor += share;
        }
    }
    placed
}

/// The node under `pointer`, or `None` when the pointer is in the empty space.
pub fn node_at(nodes: &[Placed], canvas: egui::Rect, pointer: egui::Pos2) -> Option<usize> {
    nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (index, at(canvas, node).distance(pointer)))
        .filter(|(_, distance)| *distance < 12.0)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(index, _)| index)
}

fn at(canvas: egui::Rect, node: &Placed) -> egui::Pos2 {
    egui::pos2(canvas.left() + node.at.0 * canvas.width(), canvas.top() + node.at.1 * canvas.height())
}

/// The badge that opens a note's connections, just above and to the right of its dot.
fn badge_rect(canvas: egui::Rect, node: &Placed) -> egui::Rect {
    let centre = at(canvas, node) + egui::vec2(11.0, -11.0);
    egui::Rect::from_center_size(centre, egui::vec2(15.0, 15.0))
}

/// A `+` or a `−`, painted: the fonts have neither glyph reliably, and a badge that renders as a
/// box is worse than no badge.
fn draw_sign(painter: &egui::Painter, rect: egui::Rect, open: bool, colour: egui::Color32) {
    let centre = rect.center();
    let half = 3.5;
    let stroke = egui::Stroke::new(1.4, colour);
    painter.line_segment([egui::pos2(centre.x - half, centre.y), egui::pos2(centre.x + half, centre.y)], stroke);
    if !open {
        painter.line_segment([egui::pos2(centre.x, centre.y - half), egui::pos2(centre.x, centre.y + half)], stroke);
    }
}

/// Draws the map and reports what the user asked for.
pub fn show(ui: &mut egui::Ui, state: &VaultState, map: &mut Map) -> Action {
    map.ensure_origin(state.active.as_deref());
    let mut action = Action::None;

    let origin = map.origin().map(Path::to_path_buf);
    let Some(origin) = origin else {
        ui.vertical_centered(|ui| {
            ui.add_space(24.0);
            ui.label(egui::RichText::new(tl!("Open a note to see its connections")).weak());
        });
        return action;
    };

    // The toolbar: where the map starts, how to start it over, and how to fold it all back.
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(name_of(state, &origin)).strong());
        ui.label(egui::RichText::new(tl!("in the middle")).small().weak());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add_enabled(!map.expanded.is_empty(), egui::Button::new(tl!("Collapse all"))).clicked() {
                map.collapse_all();
            }
            if ui.add_enabled(state.active.as_deref() != Some(origin.as_path()), egui::Button::new(tl!("Start from the open note"))).clicked()
                && let Some(active) = state.active.clone()
            {
                action = Action::Start(active);
            }
        });
    });
    ui.separator();

    let nodes = layout(state, map);
    if nodes.is_empty() {
        ui.label(egui::RichText::new(tl!("This note is not in the vault any more")).weak());
        return action;
    }
    if nodes.len() == 1 && !nodes[0].has_more {
        ui.add_space(6.0);
        ui.label(egui::RichText::new(tl!("This note has no links yet. Write one with [[double brackets]].")).weak());
    }
    ui.label(egui::RichText::new(tl!("Click a note to open it; the + opens its connections.")).small().weak());

    let size = ui.available_size();
    let (response, painter) = ui.allocate_painter(size, egui::Sense::click());
    let canvas = response.rect.shrink2(egui::vec2(70.0, 34.0));
    let hovered = response.hover_pos().and_then(|pointer| node_at(&nodes, canvas, pointer));
    let hovered_badge = response.hover_pos().filter(|pointer| nodes.iter().any(|node| badge_rect(canvas, node).contains(*pointer)));

    let visuals = ui.visuals().clone();
    let accent = crate::theme::accent(visuals.dark_mode);
    let faint = visuals.weak_text_color();
    let text = visuals.strong_text_color();

    // Edges first, so the nodes sit on top of them.
    let index_of: BTreeMap<&PathBuf, usize> = nodes.iter().enumerate().map(|(index, node)| (&node.path, index)).collect();
    let mut drawn: BTreeSet<(usize, usize)> = BTreeSet::new();
    for (index, node) in nodes.iter().enumerate() {
        let Some(parent) = node.parent.as_ref().and_then(|parent| index_of.get(parent)) else { continue };
        drawn.insert((*parent.min(&index), *parent.max(&index)));
        let lit = hovered == Some(index) || hovered == Some(*parent);
        painter.line_segment(
            [at(canvas, &nodes[*parent]), at(canvas, node)],
            egui::Stroke::new(if lit { 1.8 } else { 1.2 }, if lit { accent } else { faint.gamma_multiply(0.7) }),
        );
    }
    // The other links between notes already on the map: real edges, drawn quietly.
    for (index, node) in nodes.iter().enumerate() {
        for other in neighbours(state, &node.path) {
            let Some(&beside) = index_of.get(&other) else { continue };
            let pair = (index.min(beside), index.max(beside));
            if index == beside || !drawn.insert(pair) {
                continue;
            }
            painter.line_segment([at(canvas, node), at(canvas, &nodes[beside])], egui::Stroke::new(1.0, faint.gamma_multiply(0.35)));
        }
    }

    for (index, node) in nodes.iter().enumerate() {
        let centre = at(canvas, node);
        let is_origin = node.level == 0;
        let lit = hovered == Some(index);
        let radius = if is_origin {
            9.0
        } else if lit {
            7.5
        } else if node.expanded {
            6.0
        } else {
            5.0
        };
        let fill = if is_origin || node.expanded {
            accent
        } else if lit {
            text
        } else {
            faint.gamma_multiply(1.5)
        };
        painter.circle_filled(centre, radius, fill);
        if is_origin {
            painter.circle_stroke(centre, radius + 3.0, egui::Stroke::new(1.2, accent.gamma_multiply(0.65)));
        }

        // The label goes outward from the middle, so neighbours on the same ring never write over
        // each other: right/left when the note is beside the centre, above/below when it is over or
        // under it.
        let colour = if lit || is_origin { text } else { faint };
        let font = egui::FontId::proportional(if lit || is_origin { 13.0 } else { 12.0 });
        let label = painter.layout_no_wrap(node.name.clone(), font, colour);
        let outward = egui::vec2(centre.x - canvas.center().x, centre.y - canvas.center().y);
        let direction = if outward.length() < 1.0 { egui::vec2(0.0, -1.0) } else { outward.normalized() };
        let (anchor, align) = if direction.x.abs() > 0.35 {
            if direction.x > 0.0 {
                (egui::pos2(centre.x + radius + 6.0, centre.y), egui::Align2::LEFT_CENTER)
            } else {
                (egui::pos2(centre.x - radius - 6.0, centre.y), egui::Align2::RIGHT_CENTER)
            }
        } else if direction.y < 0.0 {
            (egui::pos2(centre.x, centre.y - radius - 5.0), egui::Align2::CENTER_BOTTOM)
        } else {
            (egui::pos2(centre.x, centre.y + radius + 5.0), egui::Align2::CENTER_TOP)
        };
        let placed = align.anchor_size(anchor, label.size());
        let corner = egui::pos2(
            placed.min.x.clamp(canvas.left(), (canvas.right() - placed.width()).max(canvas.left())),
            placed.min.y.clamp(canvas.top(), (canvas.bottom() - placed.height()).max(canvas.top())),
        );
        painter.galley(corner, label, colour);

        // The badge: only where there is something to open or to close.
        if is_origin || node.has_more || node.expanded {
            let badge = badge_rect(canvas, node);
            painter.circle_filled(badge.center(), 7.0, visuals.faint_bg_color);
            painter.circle_stroke(badge.center(), 7.0, egui::Stroke::new(1.0, faint.gamma_multiply(0.8)));
            draw_sign(&painter, badge, node.expanded, if hovered_badge.is_some_and(|p| badge.contains(p)) { text } else { faint });
        }

        // The interaction: a button named after the note, and the badge as a second one.
        let hit = egui::Rect::from_center_size(centre, egui::vec2(radius * 2.0 + 8.0, radius * 2.0 + 8.0));
        let node_id = ui.make_persistent_id(("map-node", &node.path));
        let clicked = ui.interact(hit, node_id, egui::Sense::click());
        clicked.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, node.name.clone()));
        if clicked.clicked() {
            action = Action::Open(node.path.clone());
        }
        clicked.context_menu(|ui| {
            if ui.button(tl!("Open")).clicked() {
                action = Action::Open(node.path.clone());
                ui.close();
            }
            let open = map.is_expanded(&node.path);
            let label = if open { tl!("Collapse") } else { tl!("Expand") };
            if ui.add_enabled(is_origin || node.has_more || open, egui::Button::new(label)).clicked() {
                map.toggle(&node.path);
                ui.close();
            }
            if ui.add_enabled(!is_origin, egui::Button::new(tl!("Start from here"))).clicked() {
                action = Action::Start(node.path.clone());
                ui.close();
            }
        });

        let badge = badge_rect(canvas, node);
        if is_origin || node.has_more || node.expanded {
            let badge_id = ui.make_persistent_id(("map-badge", &node.path));
            let hit = ui.interact(badge, badge_id, egui::Sense::click());
            let name = if node.expanded { tl!("Close the connections of {name}") } else { tl!("Open the connections of {name}") };
            let name = name.replace("{name}", &node.name);
            hit.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, name.clone()));
            if hit.clicked() {
                map.toggle(&node.path);
            }
        }
    }

    if hovered.is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    action
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A vault in memory: notes by name, with the links the map walks.
    fn vault(notes: &[(&str, &str)]) -> VaultState {
        let mut state = VaultState::default();
        state.files = notes
            .iter()
            .map(|(name, _)| localobsidian_vault::NoteFile {
                name: (*name).to_string(),
                path: PathBuf::from(format!("/v/{name}.md")),
                relative_path: format!("{name}.md"),
                mtime_ms: 0,
            })
            .collect();
        let contents: BTreeMap<PathBuf, String> =
            notes.iter().map(|(name, text)| (PathBuf::from(format!("/v/{name}.md")), (*text).to_string())).collect();
        state.index = localobsidian_vault::index::build(&state.files, &contents);
        state.replace_contents_for_tests(contents);
        state
    }

    fn path(name: &str) -> PathBuf {
        PathBuf::from(format!("/v/{name}.md"))
    }

    fn names(nodes: &[Placed]) -> Vec<String> {
        nodes.iter().map(|node| node.name.clone()).collect()
    }

    #[test]
    fn the_map_starts_with_the_note_and_its_direct_connections() {
        // `Moc` links to three notes; `Ana` links back to `Moc`. The map starts at `Moc` and shows
        // the four, and only the four: `Fundo` is two hops away and waits for a click.
        let state = vault(&[("Moc", "[[Ana]] [[Bia]] [[Caio]]"), ("Ana", "[[Moc]]"), ("Bia", "nada"), ("Caio", "[[Fundo]]"), ("Fundo", "nada")]);
        let mut map = Map::default();
        map.start_from(&path("Moc"));
        let nodes = layout(&state, &map);
        assert_eq!(names(&nodes).len(), 4, "{:?}", names(&nodes));
        assert!(!nodes.iter().any(|node| node.name == "Fundo"), "two hops away is not drawn yet");
        assert!(nodes.iter().all(|node| node.level <= 1));
        assert_eq!(nodes[0].name, "Moc", "the origin is the first node");
        assert_eq!(nodes[0].at, (0.5, 0.5), "and it sits in the middle");
    }

    #[test]
    fn expanding_a_note_adds_its_connections_and_collapsing_takes_them_away() {
        let state = vault(&[("Moc", "[[Ana]] [[Bia]]"), ("Ana", "[[Moc]]"), ("Bia", "[[Caio]]"), ("Caio", "nada")]);
        let mut map = Map::default();
        map.start_from(&path("Moc"));
        assert_eq!(layout(&state, &map).len(), 3);

        map.toggle(&path("Bia"));
        let grown = layout(&state, &map);
        assert_eq!(names(&grown).len(), 4, "{:?}", names(&grown));
        let caio = grown.iter().find(|node| node.name == "Caio").expect("the new note is on the map");
        assert_eq!(caio.level, 2, "one hop further from the middle");
        assert_eq!(caio.parent.as_deref(), Some(path("Bia").as_path()), "and it hangs from the note that was opened");

        map.toggle(&path("Bia"));
        assert_eq!(layout(&state, &map).len(), 3, "collapsing takes the branch with it");
    }

    #[test]
    fn collapsing_everything_returns_to_the_note_in_the_middle() {
        let state = vault(&[("Moc", "[[Ana]]"), ("Ana", "[[Bia]]"), ("Bia", "[[Caio]]"), ("Caio", "nada")]);
        let mut map = Map::default();
        map.start_from(&path("Moc"));
        map.toggle(&path("Ana"));
        map.toggle(&path("Bia"));
        assert_eq!(layout(&state, &map).len(), 4);
        map.collapse_all();
        assert_eq!(layout(&state, &map).len(), 2);
    }

    #[test]
    fn the_origin_stays_open_and_cannot_be_toggled() {
        let state = vault(&[("Moc", "[[Ana]]"), ("Ana", "nada")]);
        let mut map = Map::default();
        map.start_from(&path("Moc"));
        map.toggle(&path("Moc"));
        assert!(map.is_expanded(&path("Moc")), "the note in the middle is always open");
        assert_eq!(layout(&state, &map).len(), 2);
    }

    #[test]
    fn the_same_state_draws_the_same_map() {
        let state = vault(&[("Moc", "[[Ana]] [[Bia]] [[Caio]]"), ("Ana", "nada"), ("Bia", "nada"), ("Caio", "nada")]);
        let mut map = Map::default();
        map.start_from(&path("Moc"));
        assert_eq!(layout(&state, &map), layout(&state, &map), "no physics, no jitter");
    }

    #[test]
    fn the_notes_around_one_note_share_the_ring_by_name() {
        let state = vault(&[("Moc", "[[Caio]] [[Ana]] [[Bia]]"), ("Ana", "nada"), ("Bia", "nada"), ("Caio", "nada")]);
        let mut map = Map::default();
        map.start_from(&path("Moc"));
        let nodes = layout(&state, &map);
        // Clockwise from the top, measured the way the layout does it, so the wrap at π is not a
        // wrong answer: Ana, Bia, Caio.
        let turn = |name: &str| {
            let node = nodes.iter().find(|node| node.name == name).expect("a node");
            ((node.at.1 - 0.5).atan2(node.at.0 - 0.5) + std::f32::consts::FRAC_PI_2).rem_euclid(std::f32::consts::TAU)
        };
        assert!(turn("Ana") < turn("Bia") && turn("Bia") < turn("Caio"), "{:?}", nodes.iter().map(|n| (n.name.clone(), n.at)).collect::<Vec<_>>());
    }

    #[test]
    fn a_note_with_no_links_is_a_map_of_one() {
        let state = vault(&[("So", "nada"), ("Outra", "nada")]);
        let mut map = Map::default();
        map.start_from(&path("So"));
        let nodes = layout(&state, &map);
        assert_eq!(names(&nodes), ["So"]);
        assert!(!nodes[0].has_more, "there is nothing to open");
    }

    #[test]
    fn a_link_in_is_a_connection_too() {
        // A note nobody links out of is still reachable the other way: the backlink is an edge.
        let state = vault(&[("Painel", "nada"), ("Nota", "[[Painel]]")]);
        let mut map = Map::default();
        map.start_from(&path("Painel"));
        assert_eq!(names(&layout(&state, &map)), ["Painel", "Nota"]);
    }

    #[test]
    fn a_map_never_grows_past_its_ceiling() {
        let mut notes: Vec<(String, String)> = Vec::new();
        let links: Vec<String> = (0..MAX_NODES + 40).map(|index| format!("[[N{index}]]")).collect();
        notes.push(("Centro".to_string(), links.join(" ")));
        for index in 0..MAX_NODES + 40 {
            notes.push((format!("N{index}"), String::new()));
        }
        let borrowed: Vec<(&str, &str)> = notes.iter().map(|(name, text)| (name.as_str(), text.as_str())).collect();
        let state = vault(&borrowed);
        let mut map = Map::default();
        map.start_from(&path("Centro"));
        assert_eq!(layout(&state, &map).len(), MAX_NODES);
    }
}
