//! Tesseract through its command-line tool (LSTM engine), reading TSV output for boxes.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::engine::{OcrEngine, OcrLine, OcrOutput};
use crate::{OcrError, OcrImage, Rect};

const NAME: &str = "tesseract";

#[derive(Debug, Clone)]
pub struct Tesseract {
    exe: PathBuf,
    /// Page segmentation mode (`--psm`): 4 = single column, 6 = uniform block,
    /// 11 = sparse text.
    psm: u8,
    /// `--user-words` file (location and commodity names), if any.
    user_words: Option<PathBuf>,
    version: String,
}

impl Tesseract {
    /// Finds the executable: `TESSERACT` env var, `PATH`, then the default Windows install.
    pub fn find() -> Option<PathBuf> {
        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Some(p) = std::env::var_os("TESSERACT") {
            candidates.push(p.into());
        }
        candidates.push("tesseract".into());
        candidates.push(r"C:\Program Files\Tesseract-OCR\tesseract.exe".into());
        candidates.into_iter().find(|c| {
            Command::new(c)
                .arg("--version")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        })
    }

    pub fn new(psm: u8, user_words: Option<&Path>) -> Result<Self, OcrError> {
        let exe = Self::find()
            .ok_or_else(|| OcrError::unavailable(NAME, "tesseract executable not found"))?;
        let out = Command::new(&exe)
            .arg("--version")
            .output()
            .map_err(|e| OcrError::unavailable(NAME, e))?;
        let text = String::from_utf8_lossy(&out.stdout).to_string()
            + &String::from_utf8_lossy(&out.stderr);
        let version = text
            .lines()
            .next()
            .unwrap_or("tesseract ?")
            .trim()
            .to_owned();
        Ok(Self {
            exe,
            psm,
            user_words: user_words.map(Path::to_owned),
            version,
        })
    }

    pub fn version(&self) -> &str {
        &self.version
    }
}

impl OcrEngine for Tesseract {
    fn name(&self) -> String {
        format!(
            "{NAME}/psm{}{}",
            self.psm,
            if self.user_words.is_some() {
                "/dict"
            } else {
                ""
            }
        )
    }

    fn recognize(&self, img: &OcrImage) -> Result<OcrOutput, OcrError> {
        let png = img.to_png().map_err(|e| OcrError::engine(NAME, e))?;
        let mut cmd = Command::new(&self.exe);
        cmd.args(["stdin", "stdout", "-l", "eng", "--oem", "1", "--psm"])
            .arg(self.psm.to_string());
        if let Some(words) = &self.user_words {
            cmd.arg("--user-words").arg(words);
        }
        cmd.arg("tsv")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn().map_err(|e| OcrError::engine(NAME, e))?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(&png)
                .map_err(|e| OcrError::engine(NAME, e))?;
        }
        let out = child
            .wait_with_output()
            .map_err(|e| OcrError::engine(NAME, e))?;
        if !out.status.success() {
            return Err(OcrError::engine(NAME, String::from_utf8_lossy(&out.stderr)));
        }
        Ok(parse_tsv(&String::from_utf8_lossy(&out.stdout)))
    }
}

/// One line being assembled from TSV word rows.
struct LineAcc {
    key: (u32, u32, u32),
    words: Vec<String>,
    bounds: (u32, u32, u32, u32),
    conf: f32,
    n: u32,
}

/// Groups TSV word rows (level 5) into lines by (block, paragraph, line).
fn parse_tsv(tsv: &str) -> OcrOutput {
    let mut lines: Vec<LineAcc> = Vec::new();
    for row in tsv.lines().skip(1) {
        let cols: Vec<&str> = row.split('\t').collect();
        if cols.len() < 12 || cols[0] != "5" {
            continue;
        }
        let num = |i: usize| cols[i].trim().parse::<i64>().unwrap_or(0).max(0) as u32;
        let text = cols[11].trim();
        if text.is_empty() {
            continue;
        }
        let key = (num(2), num(3), num(4));
        let (x, y, w, h) = (num(6), num(7), num(8), num(9));
        let conf = cols[10].trim().parse::<f32>().unwrap_or(0.0) / 100.0;
        match lines.iter_mut().find(|l| l.key == key) {
            Some(l) => {
                l.words.push(text.to_owned());
                let b = l.bounds;
                l.bounds = (b.0.min(x), b.1.min(y), b.2.max(x + w), b.3.max(y + h));
                l.conf += conf;
                l.n += 1;
            }
            None => lines.push(LineAcc {
                key,
                words: vec![text.to_owned()],
                bounds: (x, y, x + w, y + h),
                conf,
                n: 1,
            }),
        }
    }
    let mut out = OcrOutput {
        lines: lines
            .into_iter()
            .map(|l| {
                let (x0, y0, x1, y1) = l.bounds;
                OcrLine {
                    text: l.words.join(" "),
                    rect: Some(Rect::new(x0, y0, x1 - x0, y1 - y0)),
                    confidence: Some(l.conf / l.n as f32),
                }
            })
            .collect(),
    };
    out.sort_reading_order();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_tsv_words_into_lines() {
        let header = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext";
        let rows = [
            "5\t1\t1\t1\t1\t1\t10\t5\t30\t12\t95\tDeliver",
            "5\t1\t1\t1\t1\t2\t45\t5\t20\t12\t90\t0/4",
            "4\t1\t1\t1\t2\t0\t10\t30\t50\t12\t-1\t",
            "5\t1\t1\t1\t2\t1\t10\t30\t50\t12\t80\tArea18.",
        ];
        let tsv = format!("{header}\n{}\n", rows.join("\n"));
        let out = parse_tsv(&tsv);
        assert_eq!(out.text(), "Deliver 0/4\nArea18.");
        assert_eq!(out.lines[0].rect, Some(Rect::new(10, 5, 55, 12)));
    }
}
