//! WASM-level golden corpus harness.
//!
//! Loads the actual `dist/*.wasm` parser artifacts into wasmtime and captures
//! the three parser ABI outputs (scope spans, retrieval chunks, call sites).
//! Snapshots live next to the corpus fixtures and are regenerated with
//! `GOLDEN_UPDATE=1`.

use std::path::Path;

use serde::{Deserialize, Serialize};
use wasmtime::{Engine, Instance, Linker, Memory, Module, Store};
use wasmtime_wasi::preview1::{self as wasi_preview1, WasiP1Ctx};
use wasmtime_wasi::WasiCtxBuilder;

const MAX_SPANS: usize = 64 * 1024;
const MAX_CHUNKS: usize = 8 * 1024;
const MAX_CALL_SITES: usize = 16 * 1024;

const SPAN_BYTES: usize = 12;
const CHUNK_BYTES_V1: usize = 104;
const CHUNK_BYTES_V2: usize = 106;
const CALL_SITE_BYTES: usize = 68;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Span {
    pub offset: u32,
    pub length: u32,
    pub scope: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Chunk {
    pub offset: u32,
    pub length: u32,
    pub label: String,
    pub kind: u8,
    pub visibility: u8,
    pub is_test: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CallSite {
    pub offset: u32,
    pub name: String,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoldenOutput {
    pub spans: Vec<Span>,
    pub retrieval: Vec<Chunk>,
    pub call_sites: Vec<CallSite>,
}

impl GoldenOutput {
    pub fn normalize(&mut self) {
        self.spans.sort();
        self.spans.dedup();
        self.retrieval.sort();
        self.retrieval.dedup();
        self.call_sites.sort();
        self.call_sites.dedup();
    }
}

pub struct GoldenParser {
    store: Store<WasiP1Ctx>,
    instance: Instance,
    memory: Memory,
}

impl GoldenParser {
    pub fn load(path: &Path) -> Result<Self, String> {
        let engine = Engine::default();
        let module = Module::from_file(&engine, path)
            .map_err(|e| format!("load {}: {e}", path.display()))?;
        let mut store = Store::new(&engine, WasiCtxBuilder::new().build_p1());
        let mut linker: Linker<WasiP1Ctx> = Linker::new(&engine);
        wasi_preview1::add_to_linker_sync(&mut linker, |cx| cx)
            .map_err(|e| format!("wasi linker: {e}"))?;
        let instance = linker
            .instantiate(&mut store, &module)
            .map_err(|e| format!("instantiate {}: {e}", path.display()))?;
        let memory = instance
            .get_memory(&mut store, "memory")
            .ok_or_else(|| format!("{} has no memory export", path.display()))?;
        const MIN_PAGES: u64 = 192; // 12.288 MB
        let current = memory.size(&store);
        if current < MIN_PAGES {
            memory
                .grow(&mut store, MIN_PAGES - current)
                .map_err(|e| format!("grow memory: {e}"))?;
        }
        let parser = Self {
            store,
            instance,
            memory,
        };
        Ok(parser)
    }

    fn call0_i32(&mut self, name: &str) -> Option<i32> {
        self.instance
            .get_typed_func::<(), i32>(&mut self.store, name)
            .ok()
            .and_then(|f| f.call(&mut self.store, ()).ok())
    }

    fn buffer_offsets(&mut self) -> (usize, usize) {
        let src = self
            .call0_i32("basalt_src_ptr")
            .filter(|v| *v > 0)
            .unwrap_or(1024 * 1024) as usize;
        let out = self
            .call0_i32("basalt_out_ptr")
            .filter(|v| *v > 0)
            .unwrap_or(6 * 1024 * 1024) as usize;
        (src, out)
    }

    fn write_src(&mut self, src_off: usize, src: &[u8]) -> Result<(), String> {
        let data = self.memory.data_mut(&mut self.store);
        if src_off + src.len() > data.len() {
            return Err(format!(
                "src buffer overflow: {} + {} > {}",
                src_off,
                src.len(),
                data.len()
            ));
        }
        data[src_off..src_off + src.len()].copy_from_slice(src);
        Ok(())
    }

    fn func_for(&mut self, base: &str, dialect: Option<&str>) -> Option<String> {
        if let Some(dialect) = dialect {
            let suffixed = format!("{base}_{dialect}");
            if self
                .instance
                .get_func(&mut self.store, &suffixed)
                .is_some()
            {
                return Some(suffixed);
            }
        }
        if self.instance.get_func(&mut self.store, base).is_some() {
            Some(base.to_string())
        } else {
            None
        }
    }

    pub fn parse(&mut self, src: &[u8], dialect: Option<&str>) -> Result<Vec<Span>, String> {
        let Some(name) = self.func_for("basalt_parse", dialect) else {
            return Ok(Vec::new());
        };
        let (src_off, out_off) = self.buffer_offsets();
        self.write_src(src_off, src)?;
        let f = self
            .instance
            .get_typed_func::<(i32, i32, i32, i32), i32>(&mut self.store, &name)
            .map_err(|e| format!("{name}: {e}"))?;
        let count = f
            .call(
                &mut self.store,
                (
                    src_off as i32,
                    src.len() as i32,
                    out_off as i32,
                    MAX_SPANS as i32,
                ),
            )
            .map_err(|e| format!("{name} trap: {e}"))?
            .max(0) as usize;
        let count = count.min(MAX_SPANS);
        let data = self.memory.data(&self.store);
        if out_off + count * SPAN_BYTES > data.len() {
            return Err("span output overflow".into());
        }
        let mut spans = Vec::with_capacity(count);
        for chunk in data[out_off..out_off + count * SPAN_BYTES].chunks_exact(SPAN_BYTES) {
            spans.push(Span {
                offset: u32::from_le_bytes(chunk[0..4].try_into().unwrap()),
                length: u32::from_le_bytes(chunk[4..8].try_into().unwrap()),
                scope: chunk[8],
            });
        }
        Ok(spans)
    }

    pub fn retrieval(&mut self, src: &[u8], dialect: Option<&str>) -> Result<Vec<Chunk>, String> {
        let v2_export = self
            .instance
            .get_func(&mut self.store, "basalt_retrieval_chunks_v2")
            .is_some();
        let (name, stride) = if let Some(name) = self.func_for("basalt_retrieval_chunks", dialect) {
            // The host only looks for the dialect-specific name, so v2 flags
            // are available only on the unsuffixed export.
            let v2 = name == "basalt_retrieval_chunks" && v2_export;
            (name, if v2 { CHUNK_BYTES_V2 } else { CHUNK_BYTES_V1 })
        } else if v2_export {
            ("basalt_retrieval_chunks_v2".to_string(), CHUNK_BYTES_V2)
        } else if self
            .instance
            .get_func(&mut self.store, "basalt_retrieval_chunks")
            .is_some()
        {
            ("basalt_retrieval_chunks".to_string(), CHUNK_BYTES_V1)
        } else {
            return Ok(Vec::new());
        };
        let (src_off, out_off) = self.buffer_offsets();
        self.write_src(src_off, src)?;
        let f = self
            .instance
            .get_typed_func::<(i32, i32, i32, i32), i32>(&mut self.store, &name)
            .map_err(|e| format!("{name}: {e}"))?;
        let count = f
            .call(
                &mut self.store,
                (
                    src_off as i32,
                    src.len() as i32,
                    out_off as i32,
                    MAX_CHUNKS as i32,
                ),
            )
            .map_err(|e| format!("{name} trap: {e}"))?
            .max(0) as usize;
        let count = count.min(MAX_CHUNKS);
        let data = self.memory.data(&self.store);
        if out_off + count * stride > data.len() {
            return Err("chunk output overflow".into());
        }
        let mut chunks = Vec::with_capacity(count);
        for rec in data[out_off..out_off + count * stride].chunks_exact(stride) {
            let label_bytes = &rec[8..8 + 95];
            let end = label_bytes
                .iter()
                .position(|&b| b == 0)
                .unwrap_or(label_bytes.len());
            let label = String::from_utf8_lossy(&label_bytes[..end]).trim().to_string();
            let flags = if stride == CHUNK_BYTES_V2 { rec[104] } else { 0 };
            chunks.push(Chunk {
                offset: u32::from_le_bytes(rec[0..4].try_into().unwrap()),
                length: u32::from_le_bytes(rec[4..8].try_into().unwrap()),
                label,
                kind: rec[103],
                visibility: flags & 0x03,
                is_test: flags & 0x04 != 0,
            });
        }
        Ok(chunks)
    }

    pub fn call_sites(&mut self, src: &[u8], dialect: Option<&str>) -> Result<Vec<CallSite>, String> {
        let Some(name) = self.func_for("basalt_call_sites", dialect) else {
            return Ok(Vec::new());
        };
        let (src_off, out_off) = self.buffer_offsets();
        self.write_src(src_off, src)?;
        let f = self
            .instance
            .get_typed_func::<(i32, i32, i32, i32), i32>(&mut self.store, &name)
            .map_err(|e| format!("{name}: {e}"))?;
        let count = f
            .call(
                &mut self.store,
                (
                    src_off as i32,
                    src.len() as i32,
                    out_off as i32,
                    MAX_CALL_SITES as i32,
                ),
            )
            .map_err(|e| format!("{name} trap: {e}"))?
            .max(0) as usize;
        let count = count.min(MAX_CALL_SITES);
        let data = self.memory.data(&self.store);
        if out_off + count * CALL_SITE_BYTES > data.len() {
            return Err("call-site output overflow".into());
        }
        let mut sites = Vec::with_capacity(count);
        for rec in data[out_off..out_off + count * CALL_SITE_BYTES].chunks_exact(CALL_SITE_BYTES) {
            let name_bytes = &rec[4..68];
            let end = name_bytes
                .iter()
                .position(|&b| b == 0)
                .unwrap_or(name_bytes.len());
            sites.push(CallSite {
                offset: u32::from_le_bytes(rec[0..4].try_into().unwrap()),
                name: String::from_utf8_lossy(&name_bytes[..end]).trim().to_string(),
            });
        }
        Ok(sites)
    }

    pub fn capture(
        &mut self,
        src: &[u8],
        dialect: Option<&str>,
    ) -> Result<GoldenOutput, String> {
        let mut out = GoldenOutput {
            spans: self.parse(src, dialect)?,
            retrieval: self.retrieval(src, dialect)?,
            call_sites: self.call_sites(src, dialect)?,
        };
        out.normalize();
        Ok(out)
    }
}

#[derive(Debug, Deserialize)]
struct Corpus {
    #[serde(default)]
    language: Vec<Language>,
}

#[derive(Debug, Deserialize)]
struct Language {
    name: String,
    artifact: String,
    #[serde(default)]
    fixtures: Vec<Fixture>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Fixture {
    Path(String),
    Detailed {
        path: String,
        #[serde(default)]
        dialect: Option<String>,
    },
}

impl Fixture {
    fn path(&self) -> &str {
        match self {
            Fixture::Path(path) => path,
            Fixture::Detailed { path, .. } => path,
        }
    }

    fn dialect(&self) -> Option<&str> {
        match self {
            Fixture::Path(_) => None,
            Fixture::Detailed { dialect, .. } => dialect.as_deref(),
        }
    }
}

pub fn corpus_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus")
}

fn load_corpus() -> Result<Corpus, String> {
    let text = std::fs::read_to_string(corpus_root().join("corpus.toml"))
        .map_err(|e| format!("read corpus.toml: {e}"))?;
    toml::from_str(&text).map_err(|e| format!("parse corpus.toml: {e}"))
}

pub fn run_corpus(dist: &Path) -> Result<(), String> {
    let corpus = load_corpus()?;
    let update = std::env::var("GOLDEN_UPDATE").is_ok_and(|v| v == "1");
    let strict = std::env::var("GOLDEN_DIST").is_ok();
    let root = corpus_root();
    let mut failures = Vec::new();

    for language in &corpus.language {
        let artifact = dist.join(&language.artifact);
        if !artifact.exists() {
            if strict {
                failures.push(format!("{}: missing artifact {}", language.name, artifact.display()));
            } else {
                eprintln!(
                    "skip {}: {} not built (set GOLDEN_DIST to enforce)",
                    language.name,
                    artifact.display()
                );
            }
            continue;
        }
        let mut parser = GoldenParser::load(&artifact)?;
        for fixture in &language.fixtures {
            let fixture_path = root.join(&language.name).join(fixture.path());
            let src = std::fs::read(&fixture_path)
                .map_err(|e| format!("read {}: {e}", fixture_path.display()))?;
            let captured = parser.capture(&src, fixture.dialect())?;
            let snapshot_name = format!(
                "{}.golden.json",
                fixture.path().replace(['/', '\\'], "_")
            );
            let snapshot = root.join(&language.name).join(snapshot_name);
            if update {
                let text = serde_json::to_string_pretty(&captured)
                    .map_err(|e| format!("serialize snapshot: {e}"))?;
                std::fs::write(&snapshot, text + "\n")
                    .map_err(|e| format!("write {}: {e}", snapshot.display()))?;
                eprintln!("updated {}", snapshot.display());
                continue;
            }
            let text = std::fs::read_to_string(&snapshot)
                .map_err(|e| format!("read {}: {e}", snapshot.display()))?;
            let expected: GoldenOutput =
                serde_json::from_str(&text).map_err(|e| format!("parse {}: {e}", snapshot.display()))?;
            if captured != expected {
                failures.push(format!(
                    "{}: {} drift\n--- expected ---\n{}\n--- actual ---\n{}",
                    language.name,
                    fixture.path(),
                    serde_json::to_string_pretty(&expected).unwrap_or_default(),
                    serde_json::to_string_pretty(&captured).unwrap_or_default(),
                ));
            }
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n\n"))
    }
}
