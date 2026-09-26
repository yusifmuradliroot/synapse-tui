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
            "name": "search", "description": "Search files for a text or regex-like pattern; returns file:line matches.",
            "parameters": { "type": "object", "properties": {
                "pattern": s("Text to search for"),
                "path": s("Directory to search, default workspace")
            }, "required": ["pattern"] }
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
                return "error: old_string not found".into();
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
            let mut out = String::new();
            let mut hits = 0;
            let mut stack = vec![root.clone()];
            while let Some(d) = stack.pop() {
                let Ok(rd) = std::fs::read_dir(&d) else {
                    continue;
                };
                for e in rd.filter_map(|e| e.ok()) {
                    let p = e.path();
                    let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
                    if is_dir {
                        let n = p
                            .file_name()
                            .map(|x| x.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        if n.starts_with('.') || n == "target" || n == "node_modules" {
                            continue;
                        }
                        stack.push(p);
                        continue;
                    }
                    let Ok(txt) = std::fs::read_to_string(&p) else {
                        continue;
                    };
                    for (i, l) in txt.lines().enumerate() {
                        if l.contains(&pat) {
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

fn shell(cmd: &str, ws: &Path, timeout: Option<i64>) -> String {
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
