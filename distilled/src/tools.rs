// Ajan tool'lari: JSON semasi (OpenRouter/OpenAI uyumlu) + calistirici.
// Yazma islemleri calisma dizinine (workspace) kilitlidir; okuma serbest.
// Uzun cikti kirpilir (context korumasi).
use serde_json::{json, Value};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

pub const MAX_OUT: usize = 60_000;

fn s(x: &str) -> Value {
    json!({ "type": "string", "description": x })
}

pub fn schema() -> Value {
    json!({ "type": "function", "function": {
        "name": "read_file",
        "description": "Read a UTF-8 text file. Optionally a line range. Returns numbered lines.",
        "parameters": { "type": "object", "properties": {
            "path": s("File path, absolute or relative to the workspace"),
            "start_line": { "type": "integer", "description": "1-based first line" },
            "end_line": { "type": "integer", "description": "1-based last line (inclusive)" }
        }, "required": ["path"] }
    }})
}

pub fn all() -> Value {
    json!([
        schema(),
        json!({ "type": "function", "function": {
            "name": "write_file", "description": "Create or overwrite a file with UTF-8 content.",
            "parameters": { "type": "object", "properties": {
                "path": s("File path, relative to the workspace"),
                "content": s("Full file content")
            }, "required": ["path", "content"] }
        }}),
        json!({ "type": "function", "function": {
            "name": "edit_file", "description": "Replace an exact substring in a file (must be unique unless replace_all).",
            "parameters": { "type": "object", "properties": {
                "path": s("File path"),
                "old_string": s("Exact text to find"),
                "new_string": s("Replacement text"),
                "replace_all": { "type": "boolean", "description": "Replace all occurrences" }
            }, "required": ["path", "old_string", "new_string"] }
        }}),
        json!({ "type": "function", "function": {
            "name": "list_dir", "description": "List a directory with sizes and type markers.",
            "parameters": { "type": "object", "properties": { "path": s("Directory path") } }
        }}),
        json!({ "type": "function", "function": {
            "name": "run_command", "description": "Run a shell command in the workspace and return stdout/stderr and exit code.",
            "parameters": { "type": "object", "properties": {
                "command": s("Shell command line"),
                "timeout_sec": { "type": "integer", "description": "Timeout, default 120, max 600" }
            }, "required": ["command"] }
        }}),
        json!({ "type": "function", "function": {
            "name": "search", "description": "Search files for a regex pattern; returns file:line matches. Respects .gitignore.",
            "parameters": { "type": "object", "properties": {
                "pattern": s("Regex: literals, . * + ? ^ $ [abc] [^a] (a|b) \\d \\w \\s"),
                "path": s("Directory to search, default workspace"),
                "include": s("Glob filter like src/**/*.rs, default all files")
            }, "required": ["pattern"] }
        }}),
        json!({ "type": "function", "function": {
            "name": "glob", "description": "Find files by glob pattern. Sorted by modification time, newest first.",
            "parameters": { "type": "object", "properties": {
                "pattern": s("Glob like **/*.rs or src/**/*.ts"),
                "path": s("Directory to search, default workspace")
            }, "required": ["pattern"] }
        }}),
        json!({ "type": "function", "function": {
            "name": "apply_patch", "description": "Apply a patch with *** markers. Supports Add/Update/Delete File and unified @@ hunks.",
            "parameters": { "type": "object", "properties": {
                "patch": s("Patch text. *** Add File: p | *** Update File: p + @@ hunks | *** Delete File: p, wrapped in *** Begin Patch / *** End Patch")
            }, "required": ["patch"] }
        }}),
        json!({ "type": "function", "function": {
            "name": "skill", "description": "Load a SKILL.md playbook by name and return its content.",
            "parameters": { "type": "object", "properties": { "name": s("Skill name, or empty to list available skills") } }
        }}),
        json!({ "type": "function", "function": {
            "name": "webfetch", "description": "Fetch a URL and return its text content (HTML stripped).",
            "parameters": { "type": "object", "properties": {
                "url": s("http(s) URL"),
                "max_chars": { "type": "integer", "description": "Max characters, default 20000" }
            }, "required": ["url"] }
        }}),
        json!({ "type": "function", "function": {
            "name": "todowrite", "description": "Replace the whole task list. Use for multi-step work: mark one in_progress at a time.",
            "parameters": { "type": "object", "properties": {
                "todos": { "type": "array", "items": { "type": "object", "properties": {
                    "content": s("Task description"),
                    "status": s("pending | in_progress | completed"),
                    "priority": s("high | medium | low")
                }, "required": ["content", "status"] } }
            }, "required": ["todos"] }
        }}),
        json!({ "type": "function", "function": {
            "name": "todoread", "description": "Read the current task list."
        }}),
        json!({ "type": "function", "function": {
            "name": "run_background", "description": "Start a shell command in the background (dev servers, long builds). Returns a job id; poll with job_output.",
            "parameters": { "type": "object", "properties": {
                "command": s("Shell command line")
            }, "required": ["command"] }
        }}),
        json!({ "type": "function", "function": {
            "name": "job_output", "description": "Read new output from a background job since the last read.",
            "parameters": { "type": "object", "properties": {
                "id": { "type": "integer", "description": "Job id from run_background" },
                "limit": { "type": "integer", "description": "Max bytes, default 8000" }
            }, "required": ["id"] }
        }}),
        json!({ "type": "function", "function": {
            "name": "job_stop", "description": "Stop a background job.",
            "parameters": { "type": "object", "properties": {
                "id": { "type": "integer", "description": "Job id" }
            }, "required": ["id"] }
        }}),
        json!({ "type": "function", "function": {
            "name": "ask_user", "description": "Ask the user a question and wait for the answer. Use when a choice changes the result, for confirmation, or missing info.",
            "parameters": { "type": "object", "properties": {
                "question": s("The question"),
                "options": { "type": "array", "items": { "type": "string" }, "description": "Suggested answers (user may type free text)" }
            }, "required": ["question"] }
        }}),
        json!({ "type": "function", "function": {
            "name": "system_info", "description": "Report OS, architecture, shell, current date and workspace contents summary."
        }}),
        json!({ "type": "function", "function": {
            "name": "git", "description": "Run a read-only git command: status, log, diff, show, branch.",
            "parameters": { "type": "object", "properties": { "args": s("git arguments, e.g. \"status --short\"") } }
        }})
    ])
}

fn trunc(s: &str) -> String {
    if s.len() <= MAX_OUT {
        return s.to_string();
    }
    let cut = s.floor_char_boundary(MAX_OUT);
    format!("{}\n... [truncated, {} bytes total]", &s[..cut], s.len())
}

pub fn resolve(ws: &Path, p: &str) -> PathBuf {
    let pb = Path::new(p);
    if pb.is_absolute() {
        pb.to_path_buf()
    } else {
        ws.join(pb)
    }
}

// Yazma icin: workspace disina cikmayi engelle.
fn resolve_write(ws: &Path, p: &str) -> Result<PathBuf, String> {
    let full = resolve(ws, p);
    let canon = |x: &Path| -> PathBuf {
        x.canonicalize().unwrap_or_else(|_| {
            let mut o = PathBuf::new();
            for c in x.components() {
                match c {
                    std::path::Component::ParentDir => {
                        o.pop();
                    }
                    std::path::Component::CurDir => {}
                    other => o.push(other.as_os_str()),
                }
            }
            o
        })
    };
    let f = canon(&full);
    let w = canon(ws);
    if !f.starts_with(&w) {
        return Err(format!(
            "refused: path is outside the workspace ({})",
            f.display()
        ));
    }
    Ok(f)
}

fn arg_str(a: &Value, k: &str) -> String {
    a.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string()
}

fn arg_i64(a: &Value, k: &str) -> Option<i64> {
    a.get(k).and_then(|x| x.as_i64())
}

// ── glob (*, **, ?, [abc]) ──────────────────────────────────────
fn seg_match(pat: &[char], s: &[char]) -> bool {
    let (mut pi, mut si) = (0usize, 0usize);
    let (mut star, mut ss) = (None, 0usize);
    while si < s.len() {
        if pi < pat.len() && (pat[pi] == '?' || pat[pi] == s[si]) {
            pi += 1;
            si += 1;
        } else if pi < pat.len() && pat[pi] == '*' {
            star = Some(pi);
            pi += 1;
            ss = si;
        } else if let Some(st) = star {
            pi = st + 1;
            ss += 1;
            si = ss;
        } else if pi < pat.len() && pat[pi] == '[' {
            let mut pj = pi + 1;
            let mut neg = false;
            if pj < pat.len() && pat[pj] == '^' {
                neg = true;
                pj += 1;
            }
            let mut hit = false;
            let mut closed = false;
            while pj < pat.len() {
                if pat[pj] == ']' && pj > pi + 1 + (neg as usize) {
                    closed = true;
                    break;
                }
                if pj + 2 < pat.len() && pat[pj + 1] == '-' && pat[pj + 2] != ']' {
                    if pat[pj] <= s[si] && s[si] <= pat[pj + 2] {
                        hit = true;
                    }
                    pj += 3;
                } else {
                    if pat[pj] == s[si] {
                        hit = true;
                    }
                    pj += 1;
                }
            }
            if !closed || hit == neg {
                return false;
            }
            pi = pj + 1;
            si += 1;
        } else {
            return false;
        }
    }
    while pi < pat.len() && pat[pi] == '*' {
        pi += 1;
    }
    pi == pat.len()
}

fn glob_segs(ps: &[&str], ss: &[&str]) -> bool {
    if ps.is_empty() {
        return ss.is_empty();
    }
    if ps[0] == "**" {
        for i in 0..=ss.len() {
            if glob_segs(&ps[1..], &ss[i..]) {
                return true;
            }
        }
        return false;
    }
    if ss.is_empty() {
        return false;
    }
    seg_match(
        &ps[0].chars().collect::<Vec<_>>(),
        &ss[0].chars().collect::<Vec<_>>(),
    ) && glob_segs(&ps[1..], &ss[1..])
}

pub fn glob_match(pat: &str, path: &str) -> bool {
    let ps: Vec<&str> = pat.split('/').collect();
    let ss: Vec<&str> = path.split('/').collect();
    glob_segs(&ps, &ss)
}

// ── .gitignore saygisi + yerlesik atlamalar ─────────────────────
struct Ignore {
    rules: Vec<(String, bool)>,
}

impl Ignore {
    fn load(ws: &Path) -> Ignore {
        let mut rules: Vec<(String, bool)> = vec![
            (".git/".into(), false),
            ("target/".into(), false),
            ("node_modules/".into(), false),
            ("dist/".into(), false),
            ("build/".into(), false),
        ];
        if let Ok(txt) = std::fs::read_to_string(ws.join(".gitignore")) {
            for line in txt.lines() {
                let l = line.trim();
                if l.is_empty() || l.starts_with('#') {
                    continue;
                }
                let (pat, neg) = match l.strip_prefix('!') {
                    Some(rest) => (rest.trim().to_string(), true),
                    None => (l.to_string(), false),
                };
                if !pat.is_empty() {
                    rules.push((pat, neg));
                }
            }
        }
        Ignore { rules }
    }
    fn skip(&self, rel: &str, is_dir: bool) -> bool {
        let mut out = false;
        for (pat, neg) in &self.rules {
            let hit = if pat.ends_with('/') {
                rel == pat.trim_end_matches('/') || rel.starts_with(pat)
            } else if pat.contains('*') || pat.contains('?') || pat.contains('[') {
                glob_match(pat, rel)
                    || rel.rsplit('/').next().is_some_and(|l| {
                        glob_match(pat.trim_start_matches("**/").trim_start_matches('/'), l)
                    })
            } else if pat.contains('/') {
                rel == *pat || rel.starts_with(&format!("{pat}/"))
            } else {
                rel == *pat
                    || rel.starts_with(&format!("{pat}/"))
                    || rel.rsplit('/').next() == Some(pat.as_str())
            };
            if hit {
                out = !neg;
            }
        }
        // dizin atlandiysa icindekiler de atlanir (cagiran stack'e eklemez)
        let _ = is_dir;
        out
    }
}

// ── mini regex: Thompson NFA (literals . * + ? ^ $ [] () | \d \w \s) ──
#[derive(Clone)]
enum Atom {
    Lit(char),
    Dot,
    Dig,
    Word,
    Space,
    Class(Vec<(char, char)>, bool),
}

const NO: usize = usize::MAX;

#[derive(Clone)]
struct St {
    atom: Option<Atom>,
    out: usize,
    out1: usize,
}

struct Nfa {
    st: Vec<St>,
    match_id: usize,
}

struct Frag {
    start: usize,
    outs: Vec<(usize, u8)>,
}

impl Nfa {
    fn emit(&mut self, atom: Option<Atom>) -> usize {
        self.st.push(St {
            atom,
            out: NO,
            out1: NO,
        });
        self.st.len() - 1
    }
    fn patch(&mut self, outs: &[(usize, u8)], to: usize) {
        for (s, which) in outs {
            if *which == 0 {
                self.st[*s].out = to;
            } else {
                self.st[*s].out1 = to;
            }
        }
    }
}

fn atom_ok(a: &Atom, c: char) -> bool {
    match a {
        Atom::Lit(x) => *x == c,
        Atom::Dot => true,
        Atom::Dig => c.is_ascii_digit(),
        Atom::Word => c.is_alphanumeric() || c == '_',
        Atom::Space => c.is_whitespace(),
        Atom::Class(rs, neg) => {
            let hit = rs.iter().any(|(l, h)| *l <= c && c <= *h);
            hit != *neg
        }
    }
}

struct Parser<'x> {
    p: &'x [char],
    i: usize,
    end: usize,
    nfa: Nfa,
}

impl<'x> Parser<'x> {
    fn alts(&mut self) -> Option<Frag> {
        let mut f = self.seq()?;
        while self.i < self.end && self.p[self.i] == '|' {
            self.i += 1;
            let g = self.seq()?;
            let s = self.nfa.emit(None);
            self.nfa.st[s].out = f.start;
            self.nfa.st[s].out1 = g.start;
            let mut outs = f.outs;
            outs.extend(g.outs);
            f = Frag { start: s, outs };
        }
        Some(f)
    }
    fn seq(&mut self) -> Option<Frag> {
        let mut first: Option<Frag> = None;
        while self.i < self.end && self.p[self.i] != ')' && self.p[self.i] != '|' {
            let g = self.quant()?;
            first = Some(match first {
                None => g,
                Some(f) => {
                    self.nfa.patch(&f.outs, g.start);
                    Frag {
                        start: f.start,
                        outs: g.outs,
                    }
                }
            });
        }
        first.or_else(|| {
            let s = self.nfa.emit(None);
            Some(Frag {
                start: s,
                outs: vec![(s, 0)],
            })
        })
    }
    fn quant(&mut self) -> Option<Frag> {
        let f = self.atom()?;
        if self.i < self.end {
            match self.p[self.i] {
                '*' => {
                    self.i += 1;
                    let s = self.nfa.emit(None);
                    self.nfa.st[s].out = f.start;
                    self.nfa.patch(&f.outs, s);
                    return Some(Frag {
                        start: s,
                        outs: vec![(s, 1)],
                    });
                }
                '+' => {
                    self.i += 1;
                    let s = self.nfa.emit(None);
                    self.nfa.st[s].out = f.start;
                    self.nfa.patch(&f.outs, s);
                    return Some(Frag {
                        start: f.start,
                        outs: vec![(s, 1)],
                    });
                }
                '?' => {
                    self.i += 1;
                    let s = self.nfa.emit(None);
                    self.nfa.st[s].out = f.start;
                    let mut outs = f.outs;
                    outs.push((s, 1));
                    return Some(Frag { start: s, outs });
                }
                _ => {}
            }
        }
        Some(f)
    }
    fn atom(&mut self) -> Option<Frag> {
        if self.i >= self.end {
            return None;
        }
        let c = self.p[self.i];
        if c == '(' {
            self.i += 1;
            let f = self.alts()?;
            if self.i < self.end && self.p[self.i] == ')' {
                self.i += 1;
            }
            return Some(f);
        }
        if c == '[' {
            let a = self.class()?;
            let s = self.nfa.emit(Some(a));
            return Some(Frag {
                start: s,
                outs: vec![(s, 0)],
            });
        }
        if c == '\\' && self.i + 1 < self.end {
            self.i += 1;
            let a = match self.p[self.i] {
                'd' => Atom::Dig,
                'w' => Atom::Word,
                's' => Atom::Space,
                x => Atom::Lit(x),
            };
            self.i += 1;
            let s = self.nfa.emit(Some(a));
            return Some(Frag {
                start: s,
                outs: vec![(s, 0)],
            });
        }
        if c == '.' {
            self.i += 1;
            let s = self.nfa.emit(Some(Atom::Dot));
            return Some(Frag {
                start: s,
                outs: vec![(s, 0)],
            });
        }
        self.i += 1;
        let s = self.nfa.emit(Some(Atom::Lit(c)));
        Some(Frag {
            start: s,
            outs: vec![(s, 0)],
        })
    }
    fn class(&mut self) -> Option<Atom> {
        self.i += 1; // [
        let mut neg = false;
        if self.i < self.end && self.p[self.i] == '^' {
            neg = true;
            self.i += 1;
        }
        let mut rs = Vec::new();
        let mut closed = false;
        while self.i < self.end {
            if self.p[self.i] == ']' {
                closed = true;
                self.i += 1;
                break;
            }
            if self.p[self.i] == '\\' && self.i + 1 < self.end {
                self.i += 1;
                rs.push((self.p[self.i], self.p[self.i]));
                self.i += 1;
            } else if self.i + 2 < self.end
                && self.p[self.i + 1] == '-'
                && self.p[self.i + 2] != ']'
            {
                rs.push((self.p[self.i], self.p[self.i + 2]));
                self.i += 3;
            } else {
                rs.push((self.p[self.i], self.p[self.i]));
                self.i += 1;
            }
        }
        if !closed {
            return None;
        }
        Some(Atom::Class(rs, neg))
    }
}

fn compile(pat: &str) -> Option<(Nfa, usize, bool, bool)> {
    let mut p: Vec<char> = pat.chars().collect();
    if p.is_empty() {
        return None;
    }
    let a_start = p[0] == '^';
    if a_start {
        p.remove(0);
    }
    let a_end = p.last() == Some(&'$');
    if a_end {
        p.pop();
    }
    if p.is_empty() {
        return None;
    }
    let mut ps = Parser {
        p: &p,
        i: 0,
        end: p.len(),
        nfa: Nfa {
            st: Vec::new(),
            match_id: 0,
        },
    };
    let f = ps.alts()?;
    if ps.i != ps.end {
        return None;
    }
    let m = ps.nfa.emit(None);
    ps.nfa.patch(&f.outs, m);
    ps.nfa.match_id = m;
    let start = f.start;
    Some((ps.nfa, start, a_start, a_end))
}

fn addstate(nfa: &Nfa, list: &mut Vec<usize>, seen: &mut [bool], s: usize) {
    if s == NO || seen[s] {
        return;
    }
    seen[s] = true;
    if nfa.st[s].atom.is_none() {
        let (o, o1) = (nfa.st[s].out, nfa.st[s].out1);
        if o != NO {
            addstate(nfa, list, seen, o);
        }
        if o1 != NO {
            addstate(nfa, list, seen, o1);
        }
    } else {
        list.push(s);
    }
    if s == nfa.match_id {
        list.push(s);
    }
}

/// Kalip metinde geciyor mu? (hatali kalip duz metne duser)
pub fn re_find(pat: &str, text: &str) -> Option<(usize, usize)> {
    let t: Vec<char> = text.chars().collect();
    let (nfa, start, a_start, a_end) = compile(pat).unwrap_or_else(|| {
        // derlenemedi: duz alt-dize ara
        let mut nfa = Nfa {
            st: Vec::new(),
            match_id: 0,
        };
        let mut prev: Option<usize> = None;
        let mut first = 0usize;
        for (k, c) in pat.chars().enumerate() {
            let s = nfa.emit(Some(Atom::Lit(c)));
            if k == 0 {
                first = s;
            }
            if let Some(pv) = prev {
                nfa.st[pv].out = s;
            }
            prev = Some(s);
        }
        let m = nfa.emit(None);
        if let Some(pv) = prev {
            nfa.st[pv].out = m;
        }
        nfa.match_id = m;
        (nfa, first, false, false)
    });
    let nst = nfa.st.len();
    let starts: Vec<usize> = if a_start {
        vec![0]
    } else {
        (0..=t.len()).collect()
    };
    for s in starts {
        let mut seen = vec![false; nst];
        let mut cur = Vec::new();
        addstate(&nfa, &mut cur, &mut seen, start);
        if cur.contains(&nfa.match_id) && (!a_end || s == t.len()) {
            return Some((s, s));
        }
        let mut j = s;
        let mut cur2 = cur;
        while j < t.len() {
            let mut nxt = Vec::new();
            let mut seen2 = vec![false; nst];
            for stx in cur2 {
                if stx == nfa.match_id {
                    continue;
                }
                if atom_ok(nfa.st[stx].atom.as_ref().unwrap(), t[j]) {
                    let o = nfa.st[stx].out;
                    if o != NO {
                        addstate(&nfa, &mut nxt, &mut seen2, o);
                    }
                }
            }
            j += 1;
            if nxt.contains(&nfa.match_id) && (!a_end || j == t.len()) {
                return Some((s, j));
            }
            if nxt.is_empty() {
                break;
            }
            cur2 = nxt;
        }
        if a_start {
            break;
        }
    }
    None
}
pub fn run(name: &str, args_json: &str, ws: &Path) -> String {
    let a: Value = serde_json::from_str(args_json).unwrap_or(Value::Object(Default::default()));
    match name {
        "read_file" => {
            let p = resolve(ws, &arg_str(&a, "path"));
            let txt = match std::fs::read_to_string(&p) {
                Ok(t) => t,
                Err(e) => return format!("error: cannot read {}: {e}", p.display()),
            };
            let lines: Vec<&str> = txt.lines().collect();
            let s0 = arg_i64(&a, "start_line").unwrap_or(1).max(1) as usize;
            let e0 = arg_i64(&a, "end_line").unwrap_or(lines.len() as i64).max(0) as usize;
            let e0 = e0.min(lines.len());
            if s0 > lines.len() {
                return format!(
                    "error: start_line {} beyond EOF ({} lines)",
                    s0,
                    lines.len()
                );
            }
            let mut out = String::new();
            for (i, l) in lines[s0 - 1..e0].iter().enumerate() {
                out.push_str(&format!("{:>5} | {}\n", s0 + i, l));
            }
            if out.is_empty() {
                out.push_str("(empty file)\n");
            }
            out.push_str(&format!("\n[{} lines total]\n", lines.len()));
            trunc(&out)
        }
        "write_file" => {
            let p = match resolve_write(ws, &arg_str(&a, "path")) {
                Ok(p) => p,
                Err(e) => return format!("error: {e}"),
            };
            let content = arg_str(&a, "content");
            if let Some(d) = p.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            match std::fs::write(&p, content.as_bytes()) {
                Ok(()) => format!("ok: wrote {} ({} bytes)", p.display(), content.len()),
                Err(e) => format!("error: write failed: {e}"),
            }
        }
        "edit_file" => {
            let p = match resolve_write(ws, &arg_str(&a, "path")) {
                Ok(p) => p,
                Err(e) => return format!("error: {e}"),
            };
            let txt = match std::fs::read_to_string(&p) {
                Ok(t) => t,
                Err(e) => return format!("error: cannot read {}: {e}", p.display()),
            };
            let old = arg_str(&a, "old_string");
            let new = arg_str(&a, "new_string");
            if old.is_empty() {
                return "error: old_string is empty".into();
            }
            let all = a
                .get("replace_all")
                .and_then(|x| x.as_bool())
                .unwrap_or(false);
            let n = txt.matches(&old).count();
            if n == 0 {
                // opencode tarzi yardim: en yakin satirlari goster
                let mut sug = String::from("error: old_string not found");
                let anchor = old
                    .lines()
                    .map(|l| l.trim())
                    .max_by_key(|l| l.len())
                    .unwrap_or("");
                if anchor.len() >= 4 {
                    let a: Vec<char> = anchor.chars().collect();
                    let mut scored: Vec<(usize, usize, String)> = Vec::new();
                    for (i, l) in txt.lines().enumerate().take(3000) {
                        let b: Vec<char> = l.chars().collect();
                        let mut best = 0usize;
                        // en uzun ortak alt-dize
                        let mut dp = vec![0usize; b.len() + 1];
                        for ac in a.iter() {
                            let mut ndp = vec![0usize; b.len() + 1];
                            for (y, bc) in b.iter().enumerate() {
                                if ac == bc {
                                    ndp[y + 1] = dp[y] + 1;
                                    if ndp[y + 1] > best {
                                        best = ndp[y + 1];
                                    }
                                }
                            }
                            dp = ndp;
                        }
                        if best >= 4 {
                            scored.push((best, i, l.trim().chars().take(120).collect()));
                        }
                    }
                    scored.sort_by_key(|x| std::cmp::Reverse(x.0));
                    for (_, i, l) in scored.iter().take(3) {
                        sug.push_str(&format!("\n  similar line {}: {}", i + 1, l));
                    }
                }
                return sug;
            }
            if n > 1 && !all {
                return format!(
                    "error: old_string is not unique ({n} matches); add context or replace_all"
                );
            }
            let out = if all {
                txt.replace(&old, &new)
            } else {
                txt.replacen(&old, &new, 1)
            };
            match std::fs::write(&p, out.as_bytes()) {
                Ok(()) => format!(
                    "ok: edited {} ({} replacement(s))",
                    p.display(),
                    if all { n } else { 1 }
                ),
                Err(e) => format!("error: write failed: {e}"),
            }
        }
        "list_dir" => {
            let p = resolve(ws, &arg_str(&a, "path"));
            let mut out = String::new();
            match std::fs::read_dir(&p) {
                Ok(rd) => {
                    let mut items: Vec<(String, u64, bool)> = rd
                        .filter_map(|e| e.ok())
                        .map(|e| {
                            let n = e.file_name().to_string_lossy().into_owned();
                            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
                            let sz = e.metadata().map(|m| m.len()).unwrap_or(0);
                            (n, sz, is_dir)
                        })
                        .collect();
                    items.sort();
                    for (n, sz, d) in items {
                        if d {
                            out.push_str(&format!("{n}/\n"));
                        } else {
                            out.push_str(&format!("{n}  ({sz} B)\n"));
                        }
                    }
                }
                Err(e) => out.push_str(&format!("error: {}: {e}\n", p.display())),
            }
            trunc(&out)
        }
        "run_command" => shell(&arg_str(&a, "command"), ws, arg_i64(&a, "timeout_sec")),
        "search" => {
            let pat = arg_str(&a, "pattern");
            let root = resolve(ws, &arg_str(&a, "path"));
            let include = arg_str(&a, "include");
            let ign = Ignore::load(ws);
            let mut out = String::new();
            let mut hits = 0;
            let mut stack = vec![root.clone()];
            while let Some(d) = stack.pop() {
                let Ok(rd) = std::fs::read_dir(&d) else {
                    continue;
                };
                for e in rd.filter_map(|e| e.ok()) {
                    let p = e.path();
                    let rel = p
                        .strip_prefix(ws)
                        .map(|x| x.to_string_lossy().replace('\\', "/"))
                        .unwrap_or_default();
                    if ign.skip(&rel, e.file_type().map(|t| t.is_dir()).unwrap_or(false)) {
                        continue;
                    }
                    if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                        stack.push(p);
                        continue;
                    }
                    if !include.is_empty() && !glob_match(&include, &rel) {
                        continue;
                    }
                    let Ok(txt) = std::fs::read_to_string(&p) else {
                        continue;
                    };
                    for (i, l) in txt.lines().enumerate() {
                        if re_find(&pat, l).is_some() {
                            hits += 1;
                            if hits > 300 {
                                out.push_str("... [stopped at 300 matches]\n");
                                return trunc(&out);
                            }
                            out.push_str(&format!(
                                "{}:{}: {}\n",
                                p.display(),
                                i + 1,
                                l.trim().chars().take(200).collect::<String>()
                            ));
                        }
                    }
                }
            }
            if hits == 0 {
                out.push_str(&format!("no matches for '{pat}'\n"));
            }
            trunc(&out)
        }
        "glob" => {
            let pat = arg_str(&a, "pattern");
            let root = resolve(ws, &arg_str(&a, "path"));
            let ign = Ignore::load(ws);
            let mut hits: Vec<(u64, String)> = Vec::new();
            let mut stack = vec![root.clone()];
            while let Some(d) = stack.pop() {
                let Ok(rd) = std::fs::read_dir(&d) else {
                    continue;
                };
                for e in rd.filter_map(|e| e.ok()) {
                    let p = e.path();
                    let rel = p
                        .strip_prefix(ws)
                        .map(|x| x.to_string_lossy().replace('\\', "/"))
                        .unwrap_or_default();
                    let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
                    if ign.skip(&rel, is_dir) {
                        continue;
                    }
                    if is_dir {
                        stack.push(p);
                        continue;
                    }
                    if glob_match(&pat, &rel) {
                        let mt = e
                            .metadata()
                            .ok()
                            .and_then(|m| m.modified().ok())
                            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                            .map(|d| d.as_secs())
                            .unwrap_or(0);
                        hits.push((mt, rel));
                        if hits.len() > 400 {
                            break;
                        }
                    }
                }
            }
            hits.sort_by_key(|h| std::cmp::Reverse(h.0));
            hits.truncate(200);
            if hits.is_empty() {
                return format!("no files match '{pat}'\n");
            }
            let mut out = String::new();
            for (_, r) in hits {
                out.push_str(&r);
                out.push('\n');
            }
            trunc(&out)
        }
        "apply_patch" => apply_patch(ws, &arg_str(&a, "patch")),
        "skill" => {
            let name = arg_str(&a, "name");
            skill_text(ws, &name)
        }
        "webfetch" => {
            let url = arg_str(&a, "url");
            let maxc = arg_i64(&a, "max_chars").unwrap_or(20000).clamp(500, 100000) as usize;
            webfetch(&url, maxc)
        }
        "todowrite" => {
            let arr = a
                .get("todos")
                .and_then(|x| x.as_array())
                .cloned()
                .unwrap_or_default();
            let mut todos = Vec::new();
            for t in arr.iter().take(50) {
                let content = t
                    .get("content")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                if content.is_empty() {
                    continue;
                }
                let status = t
                    .get("status")
                    .and_then(|x| x.as_str())
                    .unwrap_or("pending");
                let status = match status {
                    "in_progress" | "completed" => status.to_string(),
                    _ => "pending".to_string(),
                };
                let prio = t
                    .get("priority")
                    .and_then(|x| x.as_str())
                    .unwrap_or("medium");
                todos.push(json!({"content": content, "status": status, "priority": prio}));
            }
            let dir = ws.join(".synapse");
            let _ = std::fs::create_dir_all(&dir);
            match std::fs::write(
                dir.join("todos.json"),
                serde_json::to_string_pretty(&todos).unwrap_or_default(),
            ) {
                Ok(()) => format!("ok: {} todo(s) recorded", todos.len()),
                Err(e) => format!("error: cannot save todos: {e}"),
            }
        }
        "todoread" => {
            let p = ws.join(".synapse").join("todos.json");
            match std::fs::read_to_string(&p) {
                Ok(t) => {
                    let v: Value = serde_json::from_str(&t).unwrap_or(json!([]));
                    let list = v.as_array().cloned().unwrap_or_default();
                    if list.is_empty() {
                        return "no todos recorded".into();
                    }
                    let mut out = String::new();
                    for (i, td) in list.iter().enumerate() {
                        let c = td.get("content").and_then(|x| x.as_str()).unwrap_or("");
                        let s = td
                            .get("status")
                            .and_then(|x| x.as_str())
                            .unwrap_or("pending");
                        out.push_str(&format!("{}. [{}] {}\n", i + 1, s, c));
                    }
                    trunc(&out)
                }
                Err(_) => "no todos recorded".into(),
            }
        }
        "run_background" | "job_output" | "job_stop" | "ask_user" => {
            "error: this tool runs in the agent loop, not standalone".into()
        }
        "system_info" => {
            let mut s = String::new();
            s.push_str(&format!("os: {}\n", std::env::consts::OS));
            s.push_str(&format!("arch: {}\n", std::env::consts::ARCH));
            s.push_str(&format!("workspace: {}\n", ws.display()));
            s.push_str(&format!(
                "date: {}\n",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0)
            ));
            if let Ok(c) = std::env::var("COMSPEC") {
                s.push_str(&format!("shell: {c}\n"));
            } else {
                s.push_str("shell: /bin/sh\n");
            }
            if let Some(h) = home_dir() {
                s.push_str(&format!("home: {}\n", h.display()));
            }
            trunc(&s)
        }
        "git" => {
            let args = arg_str(&a, "args");
            let mut c = Command::new("git");
            c.current_dir(ws);
            for part in args.split_whitespace() {
                c.arg(part);
            }
            match c.output() {
                Ok(o) => {
                    let mut s = String::from_utf8_lossy(&o.stdout).into_owned();
                    s.push_str(&String::from_utf8_lossy(&o.stderr));
                    s.push_str(&format!("\n[exit {}]", o.status.code().unwrap_or(-1)));
                    trunc(&s)
                }
                Err(e) => format!("error: git not available: {e}"),
            }
        }
        other => format!("error: unknown tool '{other}'"),
    }
}

pub fn home_dir() -> Option<PathBuf> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .map(PathBuf::from)
}

// ── apply_patch ─────────────────────────────────────────────────
// Format:
//   *** Begin Patch
//   *** Add File: path        + content lines
//   *** Update File: path     + @@ hunks (or full new content if no @@)
//   *** Delete File: path
//   *** End Patch
// Unified hunks: "  ctx", "-old", "+new", "@@ -a,b +c,d @@".
pub fn apply_patch(ws: &Path, patch: &str) -> String {
    let mut out = Vec::new();
    let mut cur_file: Option<String> = None;
    let mut cur_new = false;
    let mut cur_hunks: Vec<Vec<String>> = Vec::new();
    let mut cur_body: Vec<String> = Vec::new();
    let mut files = 0;
    let flush_file = |file: &mut Option<String>,
                      is_new: &mut bool,
                      hunks: &mut Vec<Vec<String>>,
                      body: &mut Vec<String>,
                      out: &mut Vec<String>,
                      files: &mut usize| {
        let Some(f) = file.take() else {
            return;
        };
        *files += 1;
        let r = apply_one(ws, &f, *is_new, hunks, body);
        out.push(r);
        hunks.clear();
        body.clear();
    };
    let mut in_hunk = false;
    for line in patch.lines() {
        let t = line.trim_end();
        if let Some(f) = t.strip_prefix("*** Add File:") {
            flush_file(
                &mut cur_file,
                &mut cur_new,
                &mut cur_hunks,
                &mut cur_body,
                &mut out,
                &mut files,
            );
            cur_file = Some(f.trim().to_string());
            cur_new = true;
            in_hunk = false;
        } else if let Some(f) = t.strip_prefix("*** Update File:") {
            flush_file(
                &mut cur_file,
                &mut cur_new,
                &mut cur_hunks,
                &mut cur_body,
                &mut out,
                &mut files,
            );
            cur_file = Some(f.trim().to_string());
            cur_new = false;
            in_hunk = false;
        } else if let Some(f) = t.strip_prefix("*** Delete File:") {
            flush_file(
                &mut cur_file,
                &mut cur_new,
                &mut cur_hunks,
                &mut cur_body,
                &mut out,
                &mut files,
            );
            let p = resolve_write_for_patch(ws, f.trim());
            match p {
                Ok(p) => match std::fs::remove_file(&p) {
                    Ok(()) => {
                        files += 1;
                        out.push(format!("deleted {}", f.trim()))
                    }
                    Err(e) => out.push(format!("error: cannot delete {}: {e}", f.trim())),
                },
                Err(e) => out.push(format!("error: {e}")),
            }
            in_hunk = false;
        } else if t.starts_with("***") {
            in_hunk = false;
        } else if t.starts_with("@@") {
            cur_hunks.push(Vec::new());
            in_hunk = true;
        } else if in_hunk {
            if let Some(h) = cur_hunks.last_mut() {
                h.push(t.to_string());
            }
        } else if cur_file.is_some() {
            cur_body.push(t.to_string());
        }
    }
    flush_file(
        &mut cur_file,
        &mut cur_new,
        &mut cur_hunks,
        &mut cur_body,
        &mut out,
        &mut files,
    );
    if files == 0 {
        return "error: no files in patch (need *** Add/Update/Delete File:)".into();
    }
    trunc(&out.join("\n"))
}

fn resolve_write_for_patch(ws: &Path, p: &str) -> Result<PathBuf, String> {
    resolve_write(ws, p)
}

fn apply_one(
    ws: &Path,
    file: &str,
    is_new: bool,
    hunks: &[Vec<String>],
    body: &[String],
) -> String {
    let p = match resolve_write(ws, file) {
        Ok(p) => p,
        Err(e) => return format!("error: {e}"),
    };
    if is_new {
        if p.exists() {
            return format!("error: {file} exists (use Update, not Add)");
        }
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let content = body.join("\n") + if body.is_empty() { "" } else { "\n" };
        return match std::fs::write(&p, content.as_bytes()) {
            Ok(()) => format!("added {file} ({} lines)", body.len()),
            Err(e) => format!("error: write failed: {e}"),
        };
    }
    let txt = match std::fs::read_to_string(&p) {
        Ok(t) => t,
        Err(e) => return format!("error: cannot read {file}: {e}"),
    };
    if hunks.is_empty() {
        // hunk yoksa govde tam yeni iceriktir
        match std::fs::write(&p, (body.join("\n") + "\n").as_bytes()) {
            Ok(()) => format!("rewrote {file} ({} lines)", body.len()),
            Err(e) => format!("error: write failed: {e}"),
        }
    } else {
        match apply_hunks(&txt, hunks) {
            Ok((out, n)) => match std::fs::write(&p, out.as_bytes()) {
                Ok(()) => format!("patched {file} ({n} hunk(s))"),
                Err(e) => format!("error: write failed: {e}"),
            },
            Err(e) => format!("error: {file}: {e}"),
        }
    }
}

fn apply_hunks(txt: &str, hunks: &[Vec<String>]) -> Result<(String, usize), String> {
    let mut lines: Vec<String> = txt.lines().map(|l| l.to_string()).collect();
    // sondaki newline'i koru
    let trailing_nl = txt.ends_with('\n');
    for (hi, h) in hunks.iter().enumerate() {
        // beklenen konumu @@ satirindan cikar (yoksa 0)
        let mut want: Option<usize> = None;
        let mut body: Vec<&str> = Vec::new();
        for l in h {
            if l.starts_with("@@") {
                // @@ -a,b +c,d @@
                let parts: Vec<&str> = l.split_whitespace().collect();
                for pt in parts {
                    if let Some(rest) = pt.strip_prefix('+') {
                        let num: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                        if let Ok(v) = num.parse::<usize>() {
                            want = Some(v.saturating_sub(1));
                        }
                    }
                }
            } else {
                body.push(l);
            }
        }
        // adayi siralari dene: once want, sonra yakinlik
        let mut cands = Vec::new();
        if let Some(w) = want {
            cands.push(w);
            for d in 1..=40usize {
                cands.push(w.saturating_sub(d));
                cands.push(w + d);
            }
        } else {
            cands.extend(0..lines.len().saturating_add(1));
        }
        let mut done = false;
        for at in cands {
            if at > lines.len() {
                continue;
            }
            if hunk_fits(&lines, at, &body) {
                lines = hunk_apply(&lines, at, &body);
                done = true;
                break;
            }
        }
        if !done {
            return Err(format!("hunk {} does not match", hi + 1));
        }
    }
    let mut out = lines.join("\n");
    if trailing_nl {
        out.push('\n');
    }
    Ok((out, hunks.len()))
}

fn hunk_fits(lines: &[String], at: usize, body: &[&str]) -> bool {
    let mut li = at;
    for l in body {
        if let Some(ctx) = l.strip_prefix(' ') {
            if li >= lines.len() || lines[li] != ctx {
                return false;
            }
            li += 1;
        } else if let Some(old) = l.strip_prefix('-') {
            if li >= lines.len() || lines[li] != old {
                return false;
            }
            li += 1;
        } else if l.starts_with('+') {
            // ekleme, ilerlemez
        } else if l.is_empty() {
            // bos satir: baglam olarak ele al
            if li >= lines.len() || !lines[li].is_empty() {
                return false;
            }
            li += 1;
        } else {
            return false;
        }
    }
    true
}

fn hunk_apply(lines: &[String], at: usize, body: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = lines[..at.min(lines.len())].to_vec();
    let mut li = at;
    for l in body {
        if let Some(ctx) = l.strip_prefix(' ') {
            out.push(ctx.to_string());
            li += 1;
        } else if l.strip_prefix('-').is_some() {
            li += 1;
        } else if let Some(add) = l.strip_prefix('+') {
            out.push(add.to_string());
        } else if l.is_empty() {
            out.push(String::new());
            li += 1;
        }
    }
    out.extend_from_slice(&lines[li.min(lines.len())..]);
    out
}

// ── skill (SKILL.md) ────────────────────────────────────────────
pub fn skill_text(ws: &Path, name: &str) -> String {
    let mut dirs = vec![ws.join(".synapse").join("skills"), ws.join("skills")];
    if let Some(h) = home_dir() {
        dirs.push(h.join(".config").join("synapse-distilled").join("skills"));
    }
    if name.trim().is_empty() {
        let mut found = Vec::new();
        for d in &dirs {
            if let Ok(rd) = std::fs::read_dir(d) {
                for e in rd.filter_map(|e| e.ok()) {
                    if e.file_type().map(|t| t.is_dir()).unwrap_or(false)
                        && e.path().join("SKILL.md").exists()
                    {
                        found.push(e.file_name().to_string_lossy().into_owned());
                    }
                }
            }
        }
        found.sort();
        found.dedup();
        if found.is_empty() {
            return "no skills installed (add <ws>/.synapse/skills/<name>/SKILL.md)".into();
        }
        return format!("skills:\n- {}\n\nskill <name> to load", found.join("\n- "));
    }
    for d in &dirs {
        let p = d.join(name).join("SKILL.md");
        if let Ok(t) = std::fs::read_to_string(&p) {
            return trunc(&format!("--- skill: {name} ---\n{t}"));
        }
    }
    format!("error: no such skill '{name}'")
}

// ── webfetch ────────────────────────────────────────────────────
pub fn webfetch(url: &str, max_chars: usize) -> String {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return "error: only http(s) URLs".into();
    }
    let resp = match ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(25)))
        .build()
        .new_agent()
        .get(url)
        .header("User-Agent", "synapse-distilled/1.0")
        .call()
    {
        Ok(r) => r,
        Err(e) => return format!("error: fetch failed: {e}"),
    };
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    if !ct.is_empty() && !ct.contains("text") && !ct.contains("json") && !ct.contains("xml") {
        return format!("error: not a text document ({ct})");
    }
    let mut body = String::new();
    match resp.into_body().into_reader().read_to_string(&mut body) {
        Ok(_) => {}
        Err(e) => return format!("error: read failed: {e}"),
    }
    // script/style bloklarini at, etiketleri sok, bosluklari toparla
    let mut s = body;
    for (a, b) in [("<script", "</script>"), ("<style", "</style>")] {
        while let Some(i) = s.find(a) {
            match s[i..].find(b) {
                Some(j) => {
                    s.replace_range(i..i + j + b.len(), " ");
                }
                None => {
                    s.truncate(i);
                    break;
                }
            }
        }
    }
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    let mut ws_run = false;
    for c in s.chars() {
        if in_tag {
            if c == '>' {
                in_tag = false;
                if !ws_run {
                    out.push(' ');
                    ws_run = true;
                }
            }
            continue;
        }
        if c == '<' {
            in_tag = true;
            continue;
        }
        if c.is_whitespace() {
            if !ws_run {
                out.push(' ');
                ws_run = true;
            }
        } else {
            out.push(c);
            ws_run = false;
        }
    }
    let out = out
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ");
    let cut = out.floor_char_boundary(max_chars.min(out.len()));
    let mut res = out[..cut].trim().to_string();
    if out.len() > cut {
        res.push_str("\n... [truncated]");
    }
    res
}

pub fn shell(cmd: &str, ws: &Path, timeout: Option<i64>) -> String {
    if cmd.trim().is_empty() {
        return "error: empty command".into();
    }
    let secs = timeout.unwrap_or(120).clamp(1, 600) as u64;
    let mut c = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.arg("/C").arg(cmd);
        c
    } else {
        let mut c = Command::new("sh");
        c.arg("-c").arg(cmd);
        c
    };
    c.current_dir(ws)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = match c.spawn() {
        Ok(c) => c,
        Err(e) => return format!("error: cannot start shell: {e}"),
    };
    let mut so = child.stdout.take();
    let mut se = child.stderr.take();
    let (tx, rx) = mpsc::channel::<(bool, Vec<u8>)>();
    let t1 = tx.clone();
    let h1 = std::thread::spawn(move || {
        let mut b = Vec::new();
        if let Some(s) = so.as_mut() {
            let _ = s.read_to_end(&mut b);
        }
        let _ = t1.send((false, b));
    });
    let h2 = std::thread::spawn(move || {
        let mut b = Vec::new();
        if let Some(s) = se.as_mut() {
            let _ = s.read_to_end(&mut b);
        }
        let _ = tx.send((true, b));
    });
    let deadline = Duration::from_secs(secs);
    let start = std::time::Instant::now();
    let mut timed_out = false;
    let mut out = String::new();
    let mut errs = String::new();
    let mut got = 0u32;
    loop {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok((is_err, b)) => {
                let txt = String::from_utf8_lossy(&b).into_owned();
                if is_err {
                    errs.push_str(&txt);
                } else {
                    out.push_str(&txt);
                }
                got += 1;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if start.elapsed() > deadline {
                    timed_out = true;
                    let _ = child.kill();
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if got >= 2 {
            break;
        }
    }
    let status = child.wait().ok();
    let _ = h1.join();
    let _ = h2.join();
    while let Ok((is_err, b)) = rx.try_recv() {
        let txt = String::from_utf8_lossy(&b).into_owned();
        if is_err {
            errs.push_str(&txt);
        } else {
            out.push_str(&txt);
        }
    }
    if !errs.is_empty() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&errs);
    }
    if timed_out {
        out.push_str(&format!("\n[timed out after {secs}s, killed]"));
    }
    out.push_str(&format!(
        "\n[exit {}]",
        status.and_then(|s| s.code()).unwrap_or(-1)
    ));
    if out.trim().is_empty() {
        out.push_str("(no output)\n");
    }
    trunc(&out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sd-test-{}-{}", crate::session::now(), tag));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn write_read_roundtrip() {
        let w = tmp("wr");
        let r = run(
            "write_file",
            r#"{"path":"a.txt","content":" satir1\nsatir2\n"}"#,
            &w,
        );
        assert!(r.starts_with("ok:"), "{r}");
        let r = run("read_file", r#"{"path":"a.txt"}"#, &w);
        assert!(r.contains("satir1") && r.contains("satir2"), "{r}");
        let _ = std::fs::remove_dir_all(&w);
    }

    #[test]
    fn read_line_range() {
        let w = tmp("range");
        run(
            "write_file",
            r#"{"path":"b.txt","content":"1\n2\n3\n4\n5"}"#,
            &w,
        );
        let r = run(
            "read_file",
            r#"{"path":"b.txt","start_line":2,"end_line":4}"#,
            &w,
        );
        assert!(
            r.contains("2") && r.contains("4") && !r.contains(" 5 "),
            "{r}"
        );
        let _ = std::fs::remove_dir_all(&w);
    }

    #[test]
    fn edit_requires_unique() {
        let w = tmp("edit");
        run("write_file", r#"{"path":"c.txt","content":"x x"}"#, &w);
        let r = run(
            "edit_file",
            r#"{"path":"c.txt","old_string":"x","new_string":"y"}"#,
            &w,
        );
        assert!(r.contains("not unique"), "{r}");
        let r = run(
            "edit_file",
            r#"{"path":"c.txt","old_string":"x","new_string":"y","replace_all":true}"#,
            &w,
        );
        assert!(r.starts_with("ok:"), "{r}");
        let c = std::fs::read_to_string(w.join("c.txt")).unwrap();
        assert_eq!(c, "y y");
        let _ = std::fs::remove_dir_all(&w);
    }

    #[test]
    fn write_outside_workspace_refused() {
        let w = tmp("escape");
        let r = run(
            "write_file",
            r#"{"path":"/etc/synapse-should-not-exist","content":"x"}"#,
            &w,
        );
        assert!(r.starts_with("error: refused"), "{r}");
        assert!(!Path::new("/etc/synapse-should-not-exist").exists());
        let _ = std::fs::remove_dir_all(&w);
    }

    #[test]
    fn run_command_captures_output() {
        let w = tmp("cmd");
        let r = run("run_command", r#"{"command":"echo synapse-ok"}"#, &w);
        assert!(r.contains("synapse-ok"), "{r}");
        assert!(r.contains("[exit 0]"), "{r}");
        let _ = std::fs::remove_dir_all(&w);
    }

    #[test]
    fn list_and_search() {
        let w = tmp("search");
        run(
            "write_file",
            r#"{"path":"d.txt","content":"needle here"}"#,
            &w,
        );
        let r = run("list_dir", r#"{"path":"."}"#, &w);
        assert!(r.contains("d.txt"), "{r}");
        let r = run("search", r#"{"pattern":"needle","path":"."}"#, &w);
        assert!(r.contains("needle here"), "{r}");
        let _ = std::fs::remove_dir_all(&w);
    }
}

#[cfg(test)]
mod ext_tests {
    use super::*;

    fn tmp2(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sd-ext-{}-{}", crate::session::now(), tag));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn glob_match_basic() {
        assert!(glob_match("**/*.rs", "src/main.rs"));
        assert!(glob_match("**/*.rs", "main.rs"));
        assert!(glob_match("src/**/*.ts", "src/a/b/c.ts"));
        assert!(!glob_match("src/**/*.ts", "src/a.js"));
        assert!(glob_match("*.md", "README.md"));
        assert!(!glob_match("*.md", "docs/README.md"));
        assert!(glob_match("test_?.py", "test_a.py"));
        assert!(!glob_match("test_?.py", "test_ab.py"));
        assert!(glob_match("[a-c]*.txt", "b1.txt"));
        assert!(!glob_match("[a-c]*.txt", "d1.txt"));
    }

    #[test]
    fn regex_subset() {
        assert!(re_find("hello", "say hello world").is_some());
        assert!(re_find("h.llo", "say hallo").is_some());
        assert!(re_find("^say", "say hi").is_some());
        assert!(re_find("^hi", "say hi").is_none());
        assert!(re_find("hi$", "say hi").is_some());
        assert!(re_find("hi$", "hi you").is_none());
        assert!(re_find("a+b", "xaab").is_some());
        assert!(re_find("a+b", "xb").is_none());
        assert!(re_find("colou?r", "color").is_some());
        assert!(re_find("colou?r", "colour").is_some());
        assert!(re_find("[0-9]+", "abc123").is_some());
        assert!(re_find("[0-9]+", "abc").is_none());
        assert!(re_find("cat|dog", "a dog!").is_some());
        assert!(re_find("cat|dog", "bird").is_none());
        assert!(re_find(r"fn\s+\w+\(", "fn  main(").is_some());
        assert!(re_find(r"\.rs$", "main.rs").is_some());
        assert!(re_find("(ab)+c", "ababc").is_some());
        assert!(re_find("(", "a(b").is_some());
    }

    #[test]
    fn search_respects_gitignore() {
        let w = tmp2("ign");
        run(
            "write_file",
            r#"{"path":".gitignore","content":"ignored/\n*.log"}"#,
            &w,
        );
        run(
            "write_file",
            r#"{"path":"ignored/x.txt","content":"needle"}"#,
            &w,
        );
        run(
            "write_file",
            r#"{"path":"keep.txt","content":"needle"}"#,
            &w,
        );
        run(
            "write_file",
            r#"{"path":"debug.log","content":"needle"}"#,
            &w,
        );
        let r = run("search", r#"{"pattern":"needle","path":"."}"#, &w);
        assert!(r.contains("keep.txt"), "{r}");
        assert!(!r.contains("ignored"), "{r}");
        assert!(!r.contains("debug.log"), "{r}");
        let r = run("glob", r#"{"pattern":"**/*.txt"}"#, &w);
        assert!(r.contains("keep.txt") && !r.contains("ignored"), "{r}");
        let _ = std::fs::remove_dir_all(&w);
    }

    #[test]
    fn apply_patch_flow() {
        let w = tmp2("patch");
        let p = r#"{"patch":"*** Begin Patch\n*** Add File: n.txt\nhello\nworld\n*** End Patch"}"#;
        let r = run("apply_patch", p, &w);
        assert!(r.contains("added n.txt"), "{r}");
        let p2 = "{\"patch\":\"*** Begin Patch\\n*** Update File: n.txt\\n@@ -1,2 +1,2 @@\\n hello\\n-world\\n+there\\n*** End Patch\"}";
        let r = run("apply_patch", p2, &w);
        assert!(r.contains("patched n.txt"), "{r}");
        assert_eq!(
            std::fs::read_to_string(w.join("n.txt")).unwrap(),
            "hello\nthere\n"
        );
        let p3 = r#"{"patch":"*** Begin Patch\n*** Delete File: n.txt\n*** End Patch"}"#;
        let r = run("apply_patch", p3, &w);
        assert!(r.contains("deleted n.txt"), "{r}");
        assert!(!w.join("n.txt").exists());
        let _ = std::fs::remove_dir_all(&w);
    }

    #[test]
    fn todo_roundtrip() {
        let w = tmp2("todo");
        let r = run(
            "todowrite",
            r#"{"todos":[{"content":"a","status":"in_progress"},{"content":"b","status":"pending"}]}"#,
            &w,
        );
        assert!(r.contains("2 todo"), "{r}");
        let r = run("todoread", "{}", &w);
        assert!(
            r.contains("[in_progress] a") && r.contains("[pending] b"),
            "{r}"
        );
        let _ = std::fs::remove_dir_all(&w);
    }

    #[test]
    fn skill_load_and_list() {
        let w = tmp2("skill");
        std::fs::create_dir_all(w.join(".synapse/skills/rust")).unwrap();
        std::fs::write(w.join(".synapse/skills/rust/SKILL.md"), "# Rust rules\n").unwrap();
        let r = run("skill", r#"{"name":"rust"}"#, &w);
        assert!(r.contains("Rust rules"), "{r}");
        let r = run("skill", r#"{"name":""}"#, &w);
        assert!(r.contains("rust"), "{r}");
        let r = run("skill", r#"{"name":"nope"}"#, &w);
        assert!(r.contains("no such skill"), "{r}");
        let _ = std::fs::remove_dir_all(&w);
    }

    #[test]
    fn edit_suggests_similar() {
        let w = tmp2("sug");
        run(
            "write_file",
            r#"{"path":"s.txt","content":"fn main() {\n  println!(\"hi\");\n}"}"#,
            &w,
        );
        let r = run(
            "edit_file",
            r#"{"path":"s.txt","old_string":"fn mian()","new_string":"x"}"#,
            &w,
        );
        assert!(r.contains("not found") && r.contains("similar line"), "{r}");
        let _ = std::fs::remove_dir_all(&w);
    }
}
