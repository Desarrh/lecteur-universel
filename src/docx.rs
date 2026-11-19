//! Lecture minimale d'un .docx (zip + XML) : titres, paragraphes, gras/italique,
//! listes, tableaux. Pas de mise en page fidèle : c'est un lecteur rapide.

use quick_xml::Reader;
use quick_xml::events::Event;
use std::io::Read;

#[derive(Clone, Default)]
pub struct Run {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
}

#[derive(Clone)]
pub enum Block {
    Para { level: u8, list: bool, runs: Vec<Run> },
    Table(Vec<Vec<String>>),
}

pub fn parse(path: &str) -> Result<Vec<Block>, String> {
    let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut z = zip::ZipArchive::new(f).map_err(|e| format!("pas un .docx valide : {e}"))?;
    let mut xml = String::new();
    z.by_name("word/document.xml").map_err(|e| e.to_string())?.read_to_string(&mut xml).map_err(|e| e.to_string())?;
    let mut r = Reader::from_str(&xml);
    let mut out = Vec::new();
    let (mut level, mut list) = (0u8, false);
    let mut runs: Vec<Run> = Vec::new();
    let mut cur = Run::default();
    let (mut in_t, mut in_p) = (false, false);
    let mut table: Option<Vec<Vec<String>>> = None;
    let mut cell = String::new();
    let mut depth_tbl = 0;
    loop {
        let ev = r.read_event().map_err(|e| e.to_string())?;
        match &ev {
            Event::Start(e) | Event::Empty(e) => {
                let n = e.local_name();
                let empty = matches!(ev, Event::Empty(_));
                match n.as_ref() {
                    b"tbl" => {
                        depth_tbl += 1;
                        if depth_tbl == 1 {
                            table = Some(Vec::new());
                        }
                    }
                    b"tr" if depth_tbl == 1 => {
                        if let Some(t) = table.as_mut() {
                            t.push(Vec::new());
                        }
                    }
                    b"tc" if depth_tbl == 1 => cell.clear(),
                    b"p" if !empty => {
                        in_p = true;
                        level = 0;
                        list = false;
                        runs.clear();
                    }
                    b"pStyle" => {
                        for a in e.attributes().flatten() {
                            if a.key.local_name().as_ref() == b"val" {
                                let v = String::from_utf8_lossy(&a.value).to_lowercase();
                                if v == "title" || v == "titre" {
                                    level = 1;
                                } else if let Some(rest) = v.strip_prefix("heading").or_else(|| v.strip_prefix("titre")) {
                                    if let Ok(d) = rest.trim().parse::<u8>() {
                                        level = d.clamp(1, 4) + 1;
                                    }
                                }
                                if v.contains("list") || v.contains("liste") {
                                    list = true;
                                }
                            }
                        }
                    }
                    b"numPr" => list = true,
                    b"r" if !empty => cur = Run::default(),
                    b"b" => cur.bold = !off(e),
                    b"i" => cur.italic = !off(e),
                    b"t" if !empty => in_t = true,
                    b"tab" => cur.text.push('\t'),
                    b"br" => cur.text.push('\n'),
                    _ => {}
                }
            }
            Event::Text(t) if in_t => {
                let s = t.decode().map(|c| c.into_owned()).unwrap_or_default();
                let s = quick_xml::escape::unescape(&s).map(|c| c.into_owned()).unwrap_or(s);
                cur.text.push_str(&s);
            }
            Event::GeneralRef(g) if in_t => {
                let n: &[u8] = &g;
                let s = match n {
                    b"amp" => "&",
                    b"lt" => "<",
                    b"gt" => ">",
                    b"quot" => "\"",
                    b"apos" => "'",
                    _ => "",
                };
                cur.text.push_str(s);
            }
            Event::End(e) => match e.local_name().as_ref() {
                b"t" => in_t = false,
                b"r" => {
                    if !cur.text.is_empty() {
                        runs.push(std::mem::take(&mut cur));
                    }
                }
                b"p" if in_p => {
                    in_p = false;
                    if depth_tbl > 0 {
                        let s: String = runs.iter().map(|r| r.text.as_str()).collect();
                        if !cell.is_empty() {
                            cell.push(' ');
                        }
                        cell.push_str(&s);
                    } else {
                        out.push(Block::Para { level, list, runs: std::mem::take(&mut runs) });
                    }
                }
                b"tc" if depth_tbl == 1 => {
                    if let Some(row) = table.as_mut().and_then(|t| t.last_mut()) {
                        row.push(std::mem::take(&mut cell));
                    }
                }
                b"tbl" => {
                    depth_tbl -= 1;
                    if depth_tbl == 0 {
                        if let Some(t) = table.take() {
                            out.push(Block::Table(t));
                        }
                    }
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(out)
}

fn off(e: &quick_xml::events::BytesStart) -> bool {
    e.attributes().flatten().any(|a| a.key.local_name().as_ref() == b"val" && matches!(a.value.as_ref(), b"0" | b"false" | b"none"))
}

fn plain(runs: &[Run]) -> String {
    runs.iter().map(|r| r.text.as_str()).collect()
}

pub fn to_txt(b: &[Block]) -> String {
    let mut s = String::new();
    for x in b {
        match x {
            Block::Para { list, runs, .. } => {
                if *list {
                    s.push_str("• ");
                }
                s.push_str(&plain(runs));
                s.push('\n');
            }
            Block::Table(t) => {
                for row in t {
                    s.push_str(&row.join("\t"));
                    s.push('\n');
                }
                s.push('\n');
            }
        }
    }
    s
}

pub fn to_md(b: &[Block]) -> String {
    let mut s = String::new();
    for x in b {
        match x {
            Block::Para { level, list, runs } => {
                if *level > 0 {
                    s.push_str(&"#".repeat(*level as usize));
                    s.push(' ');
                } else if *list {
                    s.push_str("- ");
                }
                for r in runs {
                    let t = r.text.trim_end();
                    if t.is_empty() {
                        s.push_str(&r.text);
                        continue;
                    }
                    let m = match (r.bold, r.italic) {
                        (true, true) => "***",
                        (true, false) => "**",
                        (false, true) => "*",
                        _ => "",
                    };
                    s.push_str(m);
                    s.push_str(t);
                    s.push_str(m);
                    s.push_str(&r.text[t.len()..]);
                }
                s.push_str("\n\n");
            }
            Block::Table(t) => {
                for (i, row) in t.iter().enumerate() {
                    s.push_str("| ");
                    s.push_str(&row.iter().map(|c| c.replace('|', "\\|")).collect::<Vec<_>>().join(" | "));
                    s.push_str(" |\n");
                    if i == 0 {
                        s.push_str(&format!("|{}\n", " --- |".repeat(row.len())));
                    }
                }
                s.push('\n');
            }
        }
    }
    s
}

pub fn esc(t: &str) -> String {
    t.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

pub fn to_html(b: &[Block], title: &str) -> String {
    let mut s = format!("<!doctype html><html lang=\"fr\"><meta charset=\"utf-8\"><title>{}</title><body style=\"font-family:Segoe UI,sans-serif;max-width:46em;margin:2em auto;line-height:1.6\">\n", esc(title));
    for x in b {
        match x {
            Block::Para { level, list, runs } => {
                let tag = match level {
                    0 if *list => "li".to_string(),
                    0 => "p".to_string(),
                    l => format!("h{}", (*l).min(6)),
                };
                s.push_str(&format!("<{tag}>"));
                for r in runs {
                    let mut t = esc(&r.text);
                    if r.bold {
                        t = format!("<b>{t}</b>");
                    }
                    if r.italic {
                        t = format!("<i>{t}</i>");
                    }
                    s.push_str(&t);
                }
                s.push_str(&format!("</{tag}>\n"));
            }
            Block::Table(t) => {
                s.push_str("<table border=\"1\" cellpadding=\"6\" style=\"border-collapse:collapse\">\n");
                for row in t {
                    s.push_str("<tr>");
                    for c in row {
                        s.push_str(&format!("<td>{}</td>", esc(c)));
                    }
                    s.push_str("</tr>\n");
                }
                s.push_str("</table>\n");
            }
        }
    }
    s.push_str("</body></html>\n");
    s
}

pub fn word_count(b: &[Block]) -> usize {
    to_txt(b).split_whitespace().count()
}
