//! Model files used by the engines: where to download them and where they live on disk.
//!
//! Downloading is the only network step (the benchmark's `fetch-models` command, or the app
//! the first time it reads a screenshot), behind the `download` feature; OCR itself runs
//! offline.

use std::path::{Path, PathBuf};

/// One downloadable file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelFile {
    /// File name inside the model directory.
    pub name: &'static str,
    pub url: &'static str,
    /// Expected SHA-256 (lowercase hex).
    pub sha256: &'static str,
}

/// ocrs detection model (rten / ONNX).
pub const OCRS_DET: ModelFile = ModelFile {
    name: "ocrs-text-detection.onnx",
    url: "https://ocrs-models.s3-accelerate.amazonaws.com/text-detection.onnx",
    sha256: "a917b23dbd9524b465df7e922641b3ff2981623df4ded5a0234004ef2fee7cfe",
};
/// ocrs recognition model.
pub const OCRS_REC: ModelFile = ModelFile {
    name: "ocrs-text-recognition.onnx",
    url: "https://ocrs-models.s3-accelerate.amazonaws.com/text-recognition.onnx",
    sha256: "86c145c2edb96c8caed5b1ebb8f44d706408922211451c309c625157dd6061c5",
};
/// PP-OCRv5 server text detection (88 MB, shared by all languages).
pub const PADDLE_DET_V5_SERVER: ModelFile = ModelFile {
    name: "ppocr-v5-server-det.onnx",
    url: concat!(
        "https://huggingface.co/monkt/paddleocr-onnx/resolve/main",
        "/detection/v5/det.onnx"
    ),
    sha256: "61824840edf6e74581898930b8091b1b2318f4b2705a2e8a40ad3de7ac480133",
};
/// PP-OCRv3 English mobile text detection (2.4 MB).
pub const PADDLE_DET_V3: ModelFile = ModelFile {
    name: "ppocr-v3-det.onnx",
    url: concat!(
        "https://huggingface.co/monkt/paddleocr-onnx/resolve/main",
        "/detection/v3/det.onnx"
    ),
    sha256: "ee40e80071ba3a320d4efda75f3e22047a7d049e9bf7bcaaf9daea23fc21b935",
};
/// PP-OCRv5 English mobile recognition.
pub const PADDLE_REC_EN_V5: ModelFile = ModelFile {
    name: "ppocr-v5-en-rec.onnx",
    url: concat!(
        "https://huggingface.co/monkt/paddleocr-onnx/resolve/main",
        "/languages/english/rec.onnx"
    ),
    sha256: "4e16deb22c4da6468bdca539b2cd3c8687825538b67109177c47d359ab994cd7",
};
/// Character dictionary of [`PADDLE_REC_EN_V5`].
pub const PADDLE_DICT_EN_V5: ModelFile = ModelFile {
    name: "ppocr-v5-en-dict.txt",
    url: concat!(
        "https://huggingface.co/monkt/paddleocr-onnx/resolve/main",
        "/languages/english/dict.txt"
    ),
    sha256: "e025a66d31f327ba0c232e03f407ae8d105e1e709e7ccb3f408aa778c24e70d6",
};
/// PP-OCRv5 Latin-script mobile recognition (more symbols than the English model).
pub const PADDLE_REC_LATIN_V5: ModelFile = ModelFile {
    name: "ppocr-v5-latin-rec.onnx",
    url: concat!(
        "https://huggingface.co/monkt/paddleocr-onnx/resolve/main",
        "/languages/latin/rec.onnx"
    ),
    sha256: "614ffc2d6d3902d360fad7f1b0dd455ee45e877069d14c4e51a99dc4ef144409",
};
pub const PADDLE_DICT_LATIN_V5: ModelFile = ModelFile {
    name: "ppocr-v5-latin-dict.txt",
    url: concat!(
        "https://huggingface.co/monkt/paddleocr-onnx/resolve/main",
        "/languages/latin/dict.txt"
    ),
    sha256: "3c0a8a79b612653c25f765271714f71281e4e955962c153e272b7b8c1d2b13ff",
};

/// Every model file any engine may need.
pub const ALL: &[ModelFile] = &[
    OCRS_DET,
    OCRS_REC,
    PADDLE_DET_V5_SERVER,
    PADDLE_DET_V3,
    PADDLE_REC_EN_V5,
    PADDLE_DICT_EN_V5,
    PADDLE_REC_LATIN_V5,
    PADDLE_DICT_LATIN_V5,
];

/// What the app uses: PP-OCRv3 mobile detection + PP-OCRv5 English recognition
/// (`paddle/v3+en`, the benchmark's recommendation).
pub const PADDLE_RECOMMENDED: &[ModelFile] = &[PADDLE_DET_V3, PADDLE_REC_EN_V5, PADDLE_DICT_EN_V5];

/// Directory holding the model files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelDir(pub PathBuf);

impl ModelDir {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self(path.into())
    }

    pub fn path(&self, file: &ModelFile) -> PathBuf {
        self.0.join(file.name)
    }

    pub fn has(&self, file: &ModelFile) -> bool {
        self.path(file).is_file()
    }

    pub fn root(&self) -> &Path {
        &self.0
    }
}

/// Default model directory: the app cache dir (`.../sc-companion/models`).
#[cfg(feature = "download")]
pub fn default_dir() -> PathBuf {
    directories::ProjectDirs::from("", "", "sc-companion")
        .map(|d| d.cache_dir().join("models"))
        .unwrap_or_else(|| std::env::temp_dir().join("sc-companion").join("models"))
}

/// Lowercase hex SHA-256 of `bytes`.
#[cfg(feature = "download")]
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Downloads the missing `files` into `dir` and checks every file against its pinned
/// SHA-256. `on_download` is called before each download.
#[cfg(feature = "download")]
pub fn fetch(
    dir: &ModelDir,
    files: &[ModelFile],
    mut on_download: impl FnMut(&ModelFile),
) -> Result<(), crate::OcrError> {
    use std::io::Read as _;

    let err = |file: &ModelFile, message: String| crate::OcrError::Download {
        file: file.name.to_owned(),
        message,
    };
    let io = |path: &Path, source| crate::OcrError::Io {
        path: path.to_owned(),
        source,
    };
    std::fs::create_dir_all(dir.root()).map_err(|e| io(dir.root(), e))?;
    for file in files {
        let path = dir.path(file);
        if !path.is_file() {
            on_download(file);
            let mut body = Vec::new();
            ureq::get(file.url)
                .call()
                .map_err(|e| err(file, format!("GET {}: {e}", file.url)))?
                .into_body()
                .with_config()
                .limit(512 * 1024 * 1024)
                .reader()
                .read_to_end(&mut body)
                .map_err(|e| err(file, e.to_string()))?;
            let hash = sha256_hex(&body);
            if hash != file.sha256 {
                return Err(err(
                    file,
                    format!("sha256 {hash} does not match pinned {}", file.sha256),
                ));
            }
            // Write then rename, so an interrupted download never leaves a partial model.
            let tmp = path.with_extension("part");
            std::fs::write(&tmp, &body).map_err(|e| io(&tmp, e))?;
            std::fs::rename(&tmp, &path).map_err(|e| io(&path, e))?;
        }
        let hash = sha256_hex(&std::fs::read(&path).map_err(|e| io(&path, e))?);
        if hash != file.sha256 {
            return Err(err(
                file,
                format!(
                    "{}: sha256 {hash} does not match pinned {}",
                    path.display(),
                    file.sha256
                ),
            ));
        }
    }
    Ok(())
}
