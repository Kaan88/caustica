//! Light path expressions (track 14): regular expressions over the events of a path, after
//! OSL's notation, each accumulated into its own AOV by the LPE variant of `trace`.
//!
//! A path's events, camera first: `D` diffuse, `G` glossy, `S` specular (a smooth dielectric's
//! reflection, or a glossy lobe with roughness below 0.1), `T` transmission (refraction), `V` a
//! volume scatter, then the event that adds light: `L` a light (an emitter or a sun) or `B` the
//! background (the sky or the HDRI). Syntax: those letters, `.` any event, `R` = `[DGS]`, sets
//! `[DG]` and `[^S]`, grouping `( )`, `|`, and `*` `+` `?`. A leading `C` (the camera) is
//! optional and whitespace is ignored. Each contribution adds to a slot when its whole event
//! string matches: `C D L` is direct diffuse light, `C D [ST]+ L` caustics.
//!
//! **Labels** (wave 3): a quoted name right after an event or a set limits it to the surfaces
//! of that name: `C D'red wall' L` is light off the red wall, `C .* L'key'` the light of the
//! light named key, and a bare `'vase'` is any event at the vase. A name is an object's or a
//! light's (case and surrounding spaces ignored), or `left wall`, `right wall`, `back wall`,
//! `floor`, `main light` and `sun`. Volume (V) and background (B) events carry no label. Up to
//! [`CLASSES`] - 1 names over all slots; each symbol is then (label class, event), class 0
//! meaning no name used in any expression.
//!
//! Expressions compile to minimal DFAs of at most [`STATES`] states, uploaded as one table.
use std::collections::{BTreeSet, HashMap};

pub const SLOTS: usize = 4;
pub const STATES: usize = 32;
/// Label classes: 0 (no label in use), then one per distinct name.
pub const CLASSES: usize = 8;
/// Symbols per state in the table: per class, the 7 events and one spare (class * 8 + event).
pub const SYMBOLS: usize = CLASSES * 8;
/// After the transitions, the table holds each material id's label class (`classes`).
pub const MATERIALS: usize = 8192;
pub const EVENTS: &str = "DGSTVLB";
/// The table's "no match possible" state.
pub const DEAD: u8 = 255;

/// Ready-made expressions for the UI: (name, expression).
pub const PRESETS: [(&str, &str); 8] = [
    ("Direct diffuse", "C D L"),
    ("Direct glossy and specular", "C [GS] L"),
    ("Indirect diffuse", "C D .+ [LB]"),
    ("Caustics", "C D [ST]+ [LB]"),
    ("Reflections and refractions", "C [ST]+ .* [LB]"),
    ("Lights and sky seen directly", "C [LB]"),
    ("Volumes", "C .* V .* [LB]"),
    ("Sky light only", "C .* B"),
];

/// A deterministic automaton: state 0 starts; `next` is DEAD where nothing can match any more.
#[derive(Debug, Clone)]
pub struct Dfa {
    pub next: Vec<[u8; SYMBOLS]>,
    pub accept: Vec<bool>,
}
impl Dfa {
    /// Whether a whole event string (like "DGL", or "D'wall'L" with labels from `labels`)
    /// matches.
    #[cfg(test)]
    pub fn matches_labelled(&self, events: &str, labels: &[String]) -> bool {
        let mut s = 0u8;
        let mut chars = events.chars().peekable();
        while let Some(c) = chars.next() {
            let Some(i) = EVENTS.find(c) else {
                return false;
            };
            let mut class = 0;
            if chars.peek() == Some(&'\'') {
                chars.next();
                let name: String = chars.by_ref().take_while(|c| *c != '\'').collect();
                class = labels.iter().position(|l| *l == name).map_or(0, |p| p + 1);
            }
            s = self.next[s as usize][class * 8 + i];
            if s == DEAD {
                return false;
            }
        }
        self.accept[s as usize]
    }
    #[cfg(test)]
    pub fn matches(&self, events: &str) -> bool {
        self.matches_labelled(events, &[])
    }
}

/// A set of symbols as a bit mask (bit class * 8 + i = EVENTS[i] at that label class).
type Set = u64;
const ALL: Set = 0x7f7f_7f7f_7f7f_7f7f;
/// Events (bit i = EVENTS[i]) at any label class.
fn any_class(events: u8) -> Set {
    events as Set * 0x0101_0101_0101_0101
}
fn class_mask(class: usize) -> Set {
    0x7f << (8 * class)
}
/// Whether a symbol is a real event (class * 8 + 7 is the spare).
fn event_symbol(sym: usize) -> bool {
    sym % 8 < 7
}
/// The label name as matched: lower case, outer spaces trimmed.
pub fn label_name(name: &str) -> String {
    name.trim().to_lowercase()
}

enum Node {
    Set(Set),
    Seq(Vec<Node>),
    Alt(Vec<Node>),
    Star(Box<Node>),
    Plus(Box<Node>),
    Maybe(Box<Node>),
}

struct Parser<'a, 'l> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
    labels: &'l mut Vec<String>,
}
impl Parser<'_, '_> {
    /// A quoted name (the opening quote already read) as its label class.
    fn label(&mut self) -> Result<usize, String> {
        let mut name = String::new();
        loop {
            match self.chars.next().ok_or("missing ' after a label")? {
                '\'' => break,
                c => name.push(c),
            }
        }
        let name = label_name(&name);
        if name.is_empty() {
            return Err("empty label ''".into());
        }
        if let Some(p) = self.labels.iter().position(|l| *l == name) {
            return Ok(p + 1);
        }
        if self.labels.len() + 1 >= CLASSES {
            return Err(format!("at most {} labels", CLASSES - 1));
        }
        self.labels.push(name);
        Ok(self.labels.len())
    }
    fn peek(&mut self) -> Option<char> {
        while self.chars.peek().is_some_and(|c| c.is_whitespace()) {
            self.chars.next();
        }
        self.chars.peek().copied()
    }
    fn next(&mut self) -> Option<char> {
        self.peek();
        self.chars.next()
    }
    fn alt(&mut self) -> Result<Node, String> {
        let mut options = vec![self.seq()?];
        while self.peek() == Some('|') {
            self.next();
            options.push(self.seq()?);
        }
        Ok(if options.len() == 1 {
            options.pop().unwrap()
        } else {
            Node::Alt(options)
        })
    }
    fn seq(&mut self) -> Result<Node, String> {
        let mut items = vec![];
        while let Some(c) = self.peek()
            && c != '|'
            && c != ')'
        {
            let mut atom = self.atom()?;
            while let Some(op) = self.peek()
                && matches!(op, '*' | '+' | '?')
            {
                self.next();
                atom = match op {
                    '*' => Node::Star(Box::new(atom)),
                    '+' => Node::Plus(Box::new(atom)),
                    _ => Node::Maybe(Box::new(atom)),
                };
            }
            items.push(atom);
        }
        Ok(Node::Seq(items))
    }
    fn atom(&mut self) -> Result<Node, String> {
        let node = self.unlabelled()?;
        // A label after the event or set (spaces between are ignored, as everywhere).
        if self.peek() == Some('\'') {
            self.chars.next();
            let class = self.label()?;
            return match node {
                Node::Set(s) => Ok(Node::Set(s & class_mask(class))),
                _ => Err("a label goes right after an event or a set".into()),
            };
        }
        Ok(node)
    }
    fn unlabelled(&mut self) -> Result<Node, String> {
        match self.next().ok_or("unexpected end")? {
            // A bare label: any event there.
            '\'' => Ok(Node::Set(ALL & class_mask(self.label()?))),
            '(' => {
                let inner = self.alt()?;
                if self.next() != Some(')') {
                    return Err("missing )".into());
                }
                Ok(inner)
            }
            '[' => {
                let negate = self.peek() == Some('^');
                if negate {
                    self.next();
                }
                let mut set: Set = 0;
                loop {
                    match self.next().ok_or("missing ]")? {
                        ']' => break,
                        c => set |= symbol(c)?,
                    }
                }
                if set == 0 {
                    return Err("empty set []".into());
                }
                Ok(Node::Set(if negate { ALL & !set } else { set }))
            }
            c => Ok(Node::Set(symbol(c)?)),
        }
    }
}
fn symbol(c: char) -> Result<Set, String> {
    let c = c.to_ascii_uppercase();
    match c {
        '.' => Ok(ALL),
        'R' => Ok(any_class(0b111)),
        'C' => Err("C (the camera) may only start the expression".into()),
        '*' | '+' | '?' => Err(format!("nothing to repeat before {c}")),
        _ => EVENTS
            .find(c)
            .map(|i| any_class(1 << i))
            .ok_or_else(|| format!("unknown event {c:?} (use D G S T V L B . R)")),
    }
}

/// Thompson NFA: per state, its epsilon moves and its event moves (set, target).
#[derive(Default)]
struct Nfa {
    eps: Vec<Vec<usize>>,
    moves: Vec<Vec<(Set, usize)>>,
}
impl Nfa {
    fn state(&mut self) -> usize {
        self.eps.push(vec![]);
        self.moves.push(vec![]);
        self.eps.len() - 1
    }
    /// Builds `n` between new states; returns (start, end).
    fn build(&mut self, n: &Node) -> (usize, usize) {
        let (a, b) = (self.state(), self.state());
        match n {
            Node::Set(s) => self.moves[a].push((*s, b)),
            Node::Seq(items) => {
                let mut at = a;
                for item in items {
                    let (s, e) = self.build(item);
                    self.eps[at].push(s);
                    at = e;
                }
                self.eps[at].push(b);
            }
            Node::Alt(options) => {
                for o in options {
                    let (s, e) = self.build(o);
                    self.eps[a].push(s);
                    self.eps[e].push(b);
                }
            }
            Node::Star(inner) | Node::Plus(inner) | Node::Maybe(inner) => {
                let (s, e) = self.build(inner);
                self.eps[a].push(s);
                self.eps[e].push(b);
                if !matches!(n, Node::Plus(_)) {
                    self.eps[a].push(b);
                }
                if !matches!(n, Node::Maybe(_)) {
                    self.eps[e].push(s);
                }
            }
        }
        (a, b)
    }
    fn closure(&self, states: impl IntoIterator<Item = usize>) -> BTreeSet<usize> {
        let mut set = BTreeSet::new();
        let mut stack: Vec<usize> = states.into_iter().collect();
        while let Some(s) = stack.pop() {
            if set.insert(s) {
                stack.extend(&self.eps[s]);
            }
        }
        set
    }
}

#[cfg(test)]
pub fn compile(expr: &str) -> Result<Dfa, String> {
    compile_labelled(expr, &mut vec![])
}
/// Compiles `expr` as slot `k` of `exprs`: its labels count together with the other slots'
/// (the table shares one set of [`CLASSES`] - 1 names).
pub fn check(exprs: &[String], k: usize, expr: &str) -> Result<Dfa, String> {
    let mut labels = vec![];
    for (j, e) in exprs.iter().enumerate().take(SLOTS) {
        if j != k && !e.trim().is_empty() {
            let _ = compile_labelled(e, &mut labels);
        }
    }
    compile_labelled(expr, &mut labels)
}
/// `compile` with the label names shared across expressions (new ones are appended).
pub fn compile_labelled(expr: &str, labels: &mut Vec<String>) -> Result<Dfa, String> {
    let before = labels.len();
    let result = compile_with(expr, labels);
    if result.is_err() {
        labels.truncate(before);
    }
    result
}
fn compile_with(expr: &str, labels: &mut Vec<String>) -> Result<Dfa, String> {
    let mut p = Parser {
        chars: expr.chars().peekable(),
        labels,
    };
    if p.peek().is_some_and(|c| c.eq_ignore_ascii_case(&'C')) {
        p.next();
    }
    if p.peek().is_none() {
        return Err("empty expression".into());
    }
    let tree = p.alt()?;
    if let Some(c) = p.next() {
        return Err(format!("unexpected {c:?}"));
    }
    let mut nfa = Nfa::default();
    let (start, end) = nfa.build(&tree);
    // Subset construction.
    let mut sets = vec![nfa.closure([start])];
    let mut index: HashMap<BTreeSet<usize>, usize> = HashMap::from([(sets[0].clone(), 0)]);
    let mut next: Vec<[Option<usize>; SYMBOLS]> = vec![];
    let mut i = 0;
    while i < sets.len() {
        let mut row = [None; SYMBOLS];
        for (sym, slot) in row.iter_mut().enumerate() {
            if !event_symbol(sym) {
                continue;
            }
            let targets = sets[i].iter().flat_map(|s| {
                nfa.moves[*s]
                    .iter()
                    .filter(move |(set, _)| set & (1 << sym) != 0)
                    .map(|(_, t)| *t)
            });
            let target = nfa.closure(targets);
            if target.is_empty() {
                continue;
            }
            // Subsets can grow exponentially (".* D . . . L"); stop long before the UI stalls.
            if !index.contains_key(&target) && sets.len() >= 4096 {
                return Err(format!("too complex (more than {STATES} states)"));
            }
            let id = *index.entry(target.clone()).or_insert_with(|| {
                sets.push(target);
                sets.len() - 1
            });
            *slot = Some(id);
        }
        next.push(row);
        i += 1;
    }
    let accept: Vec<bool> = sets.iter().map(|s| s.contains(&end)).collect();
    let dfa = minimize(&next, &accept)?;
    // Contributions are matched with their light event last, so nothing else can ever match.
    let light = |n: u8| n != DEAD && dfa.accept[n as usize];
    if !dfa
        .next
        .iter()
        .any(|row| (0..CLASSES).any(|c| light(row[c * 8 + 5]) || light(row[c * 8 + 6])))
    {
        return Err("never ends in L or B (the light or the background)".into());
    }
    Ok(dfa)
}

/// Moore partition refinement, then drops states that can never accept (they become DEAD).
fn minimize(next: &[[Option<usize>; SYMBOLS]], accept: &[bool]) -> Result<Dfa, String> {
    let n = next.len();
    // Live states: those from which an accepting state is reachable.
    let mut live: Vec<bool> = accept.to_vec();
    loop {
        let mut changed = false;
        for s in 0..n {
            if !live[s] && next[s].iter().flatten().any(|t| live[*t]) {
                live[s] = true;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    if !live[0] {
        return Err("matches no path".into());
    }
    let target = |s: usize, sym: usize| next[s][sym].filter(|t| live[*t]);
    let mut class: Vec<usize> = (0..n).map(|s| accept[s] as usize).collect();
    loop {
        let mut signatures: HashMap<(usize, [Option<usize>; SYMBOLS]), usize> = HashMap::new();
        let mut refined = vec![0; n];
        for s in (0..n).filter(|s| live[*s]) {
            let sig = (
                class[s],
                std::array::from_fn(|sym| target(s, sym).map(|t| class[t])),
            );
            let count = signatures.len();
            refined[s] = *signatures.entry(sig).or_insert(count);
        }
        let stable = signatures.len() == {
            let mut seen = BTreeSet::new();
            (0..n).filter(|s| live[*s]).for_each(|s| {
                seen.insert(class[s]);
            });
            seen.len()
        };
        class = refined;
        if stable {
            break;
        }
    }
    // Renumber so the start state's class is 0.
    let mut order: HashMap<usize, u8> = HashMap::new();
    let mut queue = vec![0usize];
    order.insert(class[0], 0);
    let mut reps = vec![0usize];
    while let Some(s) = queue.pop() {
        for sym in 0..SYMBOLS {
            if let Some(t) = target(s, sym)
                && !order.contains_key(&class[t])
            {
                if order.len() >= STATES {
                    return Err(format!("too complex (more than {STATES} states)"));
                }
                order.insert(class[t], order.len() as u8);
                reps.push(t);
                queue.push(t);
            }
        }
    }
    let mut dfa = Dfa {
        next: vec![[DEAD; SYMBOLS]; reps.len()],
        accept: vec![false; reps.len()],
    };
    for (id, s) in reps.iter().enumerate() {
        dfa.accept[id] = accept[*s];
        for sym in 0..SYMBOLS {
            if let Some(t) = target(*s, sym) {
                dfa.next[id][sym] = order[&class[t]];
            }
        }
    }
    Ok(dfa)
}

/// The GPU table: per slot, state and symbol, the next state | its accept flag << 8 (DEAD and
/// unused slots never accept); `classes` follows it in the buffer. Also the number of slots in
/// use (trailing empty ones dropped) and the label names (class - 1).
pub fn table(exprs: &[String]) -> (Vec<u32>, u32, Vec<String>) {
    let mut t = vec![DEAD as u32; SLOTS * STATES * SYMBOLS];
    let mut used = 0;
    let mut labels = vec![];
    for (k, e) in exprs.iter().enumerate().take(SLOTS) {
        if e.trim().is_empty() {
            continue;
        }
        let Ok(dfa) = compile_labelled(e, &mut labels) else {
            continue;
        };
        used = k as u32 + 1;
        for (s, row) in dfa.next.iter().enumerate() {
            for (sym, n) in row.iter().enumerate() {
                let accept = *n != DEAD && dfa.accept[*n as usize];
                t[(k * STATES + s) * SYMBOLS + sym] = *n as u32 | (accept as u32) << 8;
            }
        }
    }
    (t, used, labels)
}

/// The name of each material id that labels can match (data-layouts.md "Material IDs"): the
/// room's walls, floor and main light, the objects, the extra lights, and the physical sky's sun.
pub fn material_names(s: &crate::scene::Settings) -> Vec<String> {
    let mut names: Vec<String> = [
        "left wall",
        "right wall",
        "back wall",
        "floor",
        "main light",
    ]
    .map(String::from)
    .into();
    names.extend(s.objects.iter().map(|o| label_name(&o.name)));
    names.extend(s.lights.iter().map(|l| label_name(&l.name)));
    names.push("sun".into());
    names
}
/// Each material id's label class (0 = none), the table's tail.
pub fn classes(labels: &[String], names: &[String]) -> Vec<u32> {
    let mut c = vec![0; MATERIALS];
    if labels.is_empty() {
        return c;
    }
    for (id, name) in names.iter().enumerate().take(MATERIALS) {
        if let Some(p) = labels.iter().position(|l| l == name) {
            c[id] = p as u32 + 1;
        }
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    fn m(expr: &str, events: &str) -> bool {
        compile(expr).unwrap().matches(events)
    }
    #[test]
    fn basic_expressions() {
        assert!(m("C D L", "DL"));
        assert!(!m("C D L", "GL"));
        assert!(!m("C D L", "DDL"));
        assert!(m("D L", "DL"));
        assert!(m("C D .* [LB]", "DGSB"));
        assert!(m("C D .* [LB]", "DL"));
        assert!(!m("C D .* [LB]", "GDL"));
        assert!(m("C [LB]", "B"));
        assert!(m("C D [ST]+ L", "DSTSL"));
        assert!(!m("C D [ST]+ L", "DL"));
        assert!(m("C [^S]+ L", "DGTL"));
        assert!(!m("C [^S]+ L", "DSL"));
        assert!(m("C (D|G)+ B", "DGDB"));
        assert!(m("C R? L", "L") && m("C R? L", "SL") && !m("C R? L", "TL"));
        assert!(m("c d l", "DL"));
    }
    #[test]
    fn minimal_and_errors() {
        assert_eq!(compile("C .* [LB]").unwrap().next.len(), 2);
        assert_eq!(compile("C (D|D)* L").unwrap().next.len(), 2);
        assert!(compile("").is_err());
        assert!(compile("C").is_err());
        assert!(compile("C D C").is_err());
        assert!(compile("C (D").is_err());
        assert!(compile("C X").is_err());
        assert!(compile("C []").is_err());
        assert!(compile("C *").is_err());
        assert!(compile(".{40}").is_err());
        assert!(compile("C D").is_err());
        assert!(compile("C D (G|)").is_err());
        // Exponential subset growth is cut off quickly.
        let start = std::time::Instant::now();
        assert!(compile(&format!("C .* D {}L", ". ".repeat(24))).is_err());
        assert!(start.elapsed().as_secs_f32() < 2.);
        // 33 exact positions need more than 32 states.
        assert!(compile(&"D".repeat(40)).is_err());
        for (_, e) in PRESETS {
            compile(e).unwrap();
        }
    }
    #[test]
    fn labels_split_the_alphabet() {
        let mut labels = vec![];
        let red = compile_labelled("C D'Red wall' L", &mut labels).unwrap();
        assert_eq!(labels, ["red wall"]);
        assert!(red.matches_labelled("D'red wall'L", &labels));
        assert!(!red.matches_labelled("DL", &labels));
        // A second expression shares the names; a bare label is any event there.
        let any = compile_labelled("C 'vase' .* L'key'", &mut labels).unwrap();
        assert_eq!(labels, ["red wall", "vase", "key"]);
        assert!(any.matches_labelled("G'vase'DL'key'", &labels));
        assert!(any.matches_labelled("T'vase'L'key'", &labels));
        assert!(!any.matches_labelled("G'vase'L", &labels));
        assert!(!any.matches_labelled("GL'key'", &labels));
        // Unlabelled events still match labelled surfaces.
        assert!(
            compile("C D L")
                .unwrap()
                .matches_labelled("D'vase'L'key'", &labels)
        );
        // Sets take labels; groups don't; a failed compile adds no names.
        assert!(
            compile("C [DG]'x' L")
                .unwrap()
                .matches_labelled("G'x'L", &["x".into()])
        );
        assert!(compile("C (D|G)'x' L").is_err());
        assert!(
            compile("C D 'x' L")
                .unwrap()
                .matches_labelled("D'x'L", &["x".into()])
        );
        // Names count over all slots.
        let four = |a: char| {
            (0..4)
                .map(|i| format!("'{a}{i}'"))
                .collect::<Vec<_>>()
                .join("|")
        };
        let slots = vec![
            format!("C ({}) L", four('a')),
            format!("C ({}) L", four('b')),
        ];
        assert!(check(&slots, 0, &slots[0]).is_err() && check(&slots[..1], 0, &slots[0]).is_ok());
        assert!(compile("C D'' L").is_err());
        assert!(compile("C D'open L").is_err());
        let mut l = vec![];
        assert!(compile_labelled("C D'a' X", &mut l).is_err() && l.is_empty());
        let many = (0..8)
            .map(|i| format!("'n{i}'"))
            .collect::<Vec<_>>()
            .join("|");
        assert!(compile(&format!("C ({many}) L")).is_err());
        // The table's tail maps material ids to classes.
        let (t, used, names) = table(&["C D'floor' L".into()]);
        assert_eq!((used, names.clone()), (1, vec!["floor".to_string()]));
        assert_eq!(t.len(), SLOTS * STATES * SYMBOLS);
        let c = classes(&names, &material_names(&crate::scene::Settings::default()));
        assert_eq!((c[3], c[0], c.len()), (1, 0, MATERIALS));
    }
    #[test]
    fn table_encodes_accepts() {
        let (t, used, _) = table(&["".into(), "C D L".into()]);
        assert_eq!(used, 2);
        let base = STATES * SYMBOLS;
        // Slot 1: D from the start, then L accepts.
        let d = t[base] as usize & 0xff;
        assert_eq!(t[base] >> 8, 0);
        let l = t[base + (d * SYMBOLS) + 5];
        assert_eq!(l >> 8, 1);
        assert_eq!(t[0], DEAD as u32);
    }
}
