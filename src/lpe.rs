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
//! Expressions compile to minimal DFAs of at most [`STATES`] states, uploaded as one table.
use std::collections::{BTreeSet, HashMap};

pub const SLOTS: usize = 4;
pub const STATES: usize = 32;
/// Symbols per state in the table: the 7 events, and one spare.
pub const SYMBOLS: usize = 8;
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
    /// Whether a whole event string (like "DGL") matches.
    #[cfg(test)]
    pub fn matches(&self, events: &str) -> bool {
        let mut s = 0u8;
        for c in events.chars() {
            let Some(i) = EVENTS.find(c) else {
                return false;
            };
            s = self.next[s as usize][i];
            if s == DEAD {
                return false;
            }
        }
        self.accept[s as usize]
    }
}

/// A set of events as a bit mask (bit i = EVENTS[i]).
type Set = u8;
const ALL: Set = (1 << 7) - 1;

enum Node {
    Set(Set),
    Seq(Vec<Node>),
    Alt(Vec<Node>),
    Star(Box<Node>),
    Plus(Box<Node>),
    Maybe(Box<Node>),
}

struct Parser<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
}
impl Parser<'_> {
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
        match self.next().ok_or("unexpected end")? {
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
                let mut set = 0;
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
        'R' => Ok(0b111),
        'C' => Err("C (the camera) may only start the expression".into()),
        '*' | '+' | '?' => Err(format!("nothing to repeat before {c}")),
        _ => EVENTS
            .find(c)
            .map(|i| 1 << i)
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

pub fn compile(expr: &str) -> Result<Dfa, String> {
    let mut p = Parser {
        chars: expr.chars().peekable(),
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
    let mut next: Vec<[Option<usize>; 7]> = vec![];
    let mut i = 0;
    while i < sets.len() {
        let mut row = [None; 7];
        for (sym, slot) in row.iter_mut().enumerate() {
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
    if !dfa.next.iter().any(|row| light(row[5]) || light(row[6])) {
        return Err("never ends in L or B (the light or the background)".into());
    }
    Ok(dfa)
}

/// Moore partition refinement, then drops states that can never accept (they become DEAD).
fn minimize(next: &[[Option<usize>; 7]], accept: &[bool]) -> Result<Dfa, String> {
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
        let mut signatures: HashMap<(usize, [Option<usize>; 7]), usize> = HashMap::new();
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
        for sym in 0..7 {
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
        for sym in 0..7 {
            if let Some(t) = target(*s, sym) {
                dfa.next[id][sym] = order[&class[t]];
            }
        }
    }
    Ok(dfa)
}

/// The GPU table: per slot, state and symbol, the next state | its accept flag << 8 (DEAD and
/// unused slots never accept). Also the number of slots in use (trailing empty ones dropped).
pub fn table(exprs: &[String]) -> (Vec<u32>, u32) {
    let mut t = vec![DEAD as u32; SLOTS * STATES * SYMBOLS];
    let mut used = 0;
    for (k, e) in exprs.iter().enumerate().take(SLOTS) {
        if e.trim().is_empty() {
            continue;
        }
        let Ok(dfa) = compile(e) else {
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
    (t, used)
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
    fn table_encodes_accepts() {
        let (t, used) = table(&["".into(), "C D L".into()]);
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
