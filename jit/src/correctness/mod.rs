//! Shared correctness-suite parsing and reporting primitives.
//!
//! The command-line runners and integration tests use this module so a suite
//! cannot silently interpret Test262 metadata differently in different modes.

use serde::{Deserialize, Serialize};
use std::{
    error::Error,
    fmt, fs, io,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParseError(String);

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Error for ParseError {}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NegativePhase {
    Parse,
    Resolution,
    Runtime,
}

#[derive(Clone, Debug, Deserialize)]
struct NegativeMetadata {
    phase: NegativePhase,
    #[serde(rename = "type")]
    error_type: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct Metadata {
    #[serde(default)]
    flags: Vec<String>,
    #[serde(default)]
    includes: Vec<String>,
    #[serde(default)]
    features: Vec<String>,
    negative: Option<NegativeMetadata>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Test262Variant {
    SloppyScript,
    StrictScript,
    RawScript,
    Module,
    StrictModule,
    AsyncScript,
    StrictAsyncScript,
    ModuleAsync,
    StrictModuleAsync,
}

#[derive(Clone, Debug)]
pub struct Negative {
    phase: NegativePhase,
    error_type: String,
}

impl Negative {
    pub const fn phase(&self) -> NegativePhase {
        self.phase
    }

    pub fn error_type(&self) -> &str {
        &self.error_type
    }
}

#[derive(Clone, Debug)]
pub struct Test262Case {
    path: String,
    body: String,
    includes: Vec<String>,
    features: Vec<String>,
    negative: Option<Negative>,
    variants: Vec<Test262Variant>,
}

impl Test262Case {
    pub fn path(&self) -> &str {
        &self.path
    }
    pub fn body(&self) -> &str {
        &self.body
    }
    pub fn includes(&self) -> &[String] {
        &self.includes
    }
    pub fn features(&self) -> &[String] {
        &self.features
    }
    pub fn negative(&self) -> Option<&Negative> {
        self.negative.as_ref()
    }
    pub fn variants(&self) -> &[Test262Variant] {
        &self.variants
    }
}

pub fn parse_test262(path: impl Into<String>, source: &str) -> Result<Test262Case, ParseError> {
    let path = path.into();
    let marker = "/*---";
    let start = source
        .find(marker)
        .ok_or_else(|| ParseError(format!("{path}: missing Test262 YAML frontmatter")))?;
    let yaml_start = start + marker.len();
    let relative_end = source[yaml_start..]
        .find("---*/")
        .ok_or_else(|| ParseError(format!("{path}: unterminated Test262 YAML frontmatter")))?;
    let yaml_end = yaml_start + relative_end;
    let metadata: Metadata = serde_yaml::from_str(&source[yaml_start..yaml_end])
        .map_err(|error| ParseError(format!("{path}: invalid Test262 YAML: {error}")))?;
    const KNOWN_FLAGS: &[&str] = &[
        "onlyStrict",
        "noStrict",
        "raw",
        "async",
        "module",
        "CanBlockIsFalse",
        "CanBlockIsTrue",
        "generated",
    ];
    for flag in &metadata.flags {
        if !KNOWN_FLAGS.contains(&flag.as_str()) {
            return Err(ParseError(format!("{path}: unknown flag {flag}")));
        }
    }
    let has = |flag: &str| metadata.flags.iter().any(|candidate| candidate == flag);
    if has("onlyStrict") && has("noStrict") || has("raw") && metadata.flags.len() != 1 {
        return Err(ParseError(format!("{path}: contradictory Test262 flags")));
    }
    let variants = if has("raw") {
        vec![Test262Variant::RawScript]
    } else {
        let module = has("module");
        let asynchronous = has("async");
        let variant = |strict| match (module, asynchronous, strict) {
            (false, false, false) => Test262Variant::SloppyScript,
            (false, false, true) => Test262Variant::StrictScript,
            (true, false, false) => Test262Variant::Module,
            (true, false, true) => Test262Variant::StrictModule,
            (false, true, false) => Test262Variant::AsyncScript,
            (false, true, true) => Test262Variant::StrictAsyncScript,
            (true, true, false) => Test262Variant::ModuleAsync,
            (true, true, true) => Test262Variant::StrictModuleAsync,
        };
        if has("onlyStrict") || module {
            vec![variant(true)]
        } else if has("noStrict") {
            vec![variant(false)]
        } else {
            vec![variant(false), variant(true)]
        }
    };
    let body_start = yaml_end + "---*/".len();
    let body = source[body_start..]
        .strip_prefix('\n')
        .unwrap_or(&source[body_start..])
        .to_owned();
    Ok(Test262Case {
        path,
        body,
        includes: metadata.includes,
        features: metadata.features,
        negative: metadata.negative.map(|negative| Negative {
            phase: negative.phase,
            error_type: negative.error_type,
        }),
        variants,
    })
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Exclusion {
    pub pattern: String,
    pub reason: String,
    pub owner: String,
    pub expires: String,
    #[serde(default)]
    pub features: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(transparent)]
pub struct ExclusionManifest(Vec<Exclusion>);

impl ExclusionManifest {
    pub fn new(entries: Vec<Exclusion>) -> Self {
        Self(entries)
    }

    pub fn validate(&self, today: &str) -> Result<(), String> {
        for entry in &self.0 {
            if entry.pattern.trim().is_empty()
                || entry.reason.trim().is_empty()
                || entry.owner.trim().is_empty()
            {
                return Err("exclusion pattern, reason, and owner must be nonempty".into());
            }
            if !valid_iso_date(&entry.expires) {
                return Err(format!(
                    "{} has invalid expiry {}",
                    entry.pattern, entry.expires
                ));
            }
            if entry.expires.as_str() < today {
                return Err(format!(
                    "{} exclusion expired on {}",
                    entry.pattern, entry.expires
                ));
            }
        }
        Ok(())
    }

    pub fn reason_for(&self, path: &str, features: &[String]) -> Option<String> {
        self.0
            .iter()
            .find(|entry| {
                let path_matches = entry.pattern == "**"
                    || entry.pattern == path
                    || entry
                        .pattern
                        .strip_suffix("**")
                        .is_some_and(|prefix| path.starts_with(prefix));
                path_matches
                    && (entry.features.is_empty()
                        || entry.features.iter().any(|feature| {
                            features.iter().any(|candidate| {
                                candidate == feature
                                    || candidate
                                        .strip_prefix(feature)
                                        .is_some_and(|suffix| suffix.starts_with('.'))
                            })
                        }))
            })
            .map(|entry| {
                format!(
                    "{} (owner: {}, expires: {})",
                    entry.reason, entry.owner, entry.expires
                )
            })
    }
    pub fn path_reason(&self, path: &str) -> Option<String> {
        self.0
            .iter()
            .find(|entry| {
                entry.features.is_empty()
                    && entry.pattern != "**"
                    && (entry.pattern == path
                        || entry
                            .pattern
                            .strip_suffix("**")
                            .is_some_and(|prefix| path.starts_with(prefix)))
            })
            .map(|entry| {
                format!(
                    "{} (owner: {}, expires: {})",
                    entry.reason, entry.owner, entry.expires
                )
            })
    }
}

fn valid_iso_date(date: &str) -> bool {
    let bytes = date.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| index == 4 || index == 7 || byte.is_ascii_digit())
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunMode {
    Interpreter,
    Automatic,
    ForceTier1,
    ForceTier2,
}

impl std::str::FromStr for RunMode {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "interpreter" => Ok(Self::Interpreter),
            "automatic" => Ok(Self::Automatic),
            "force-tier1" | "force-tier1-eligible" => Ok(Self::ForceTier1),
            "force-tier2" | "force-tier2-eligible" => Ok(Self::ForceTier2),
            _ => Err(format!("unknown strict runtime mode {value}")),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CaseStatus {
    Pass,
    Fail,
    Skip,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct NativeEvidence {
    pub native_entries: u64,
    pub tier2_entries: u64,
    pub native_exits: u64,
    pub unexpected_fallbacks: u64,
    pub opcode_ids: Vec<u16>,
    pub helper_ids: Vec<u16>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CaseReport {
    pub path: String,
    pub variant: Test262Variant,
    pub status: CaseStatus,
    pub duration_ms: u64,
    pub negative_phase: Option<NegativePhase>,
    pub negative_type: Option<String>,
    pub skip_reason: Option<String>,
    pub error: Option<String>,
    pub native: NativeEvidence,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SuiteReport {
    pub schema_version: u32,
    pub suite: String,
    pub suite_revision: String,
    pub opcode_fingerprint: u64,
    pub mode: RunMode,
    pub discovered_files: usize,
    pub shard_index: usize,
    pub shard_count: usize,
    pub cases: Vec<CaseReport>,
}

impl SuiteReport {
    pub fn new(
        suite: impl Into<String>,
        suite_revision: impl Into<String>,
        mode: RunMode,
        cases: Vec<CaseReport>,
    ) -> Self {
        Self {
            schema_version: 1,
            suite: suite.into(),
            suite_revision: suite_revision.into(),
            opcode_fingerprint: crate::abi::OPCODE_FINGERPRINT,
            mode,
            discovered_files: cases.len(),
            shard_index: 0,
            shard_count: 1,
            cases,
        }
    }
    pub fn with_discovery(
        mut self,
        discovered_files: usize,
        shard_index: usize,
        shard_count: usize,
    ) -> Self {
        self.discovered_files = discovered_files;
        self.shard_index = shard_index;
        self.shard_count = shard_count;
        self
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.cases.is_empty() {
            return Err(format!("{} report contains zero cases", self.suite));
        }
        if self.discovered_files == 0
            || self.shard_count == 0
            || self.shard_index >= self.shard_count
        {
            return Err("invalid discovery/shard metadata".into());
        }
        if matches!(self.mode, RunMode::ForceTier1 | RunMode::ForceTier2) {
            for case in self
                .cases
                .iter()
                .filter(|case| matches!(case.status, CaseStatus::Pass))
            {
                if case.native.native_entries == 0 {
                    return Err(format!(
                        "{} {:?} passed forced mode without native entry",
                        case.path, case.variant
                    ));
                }
                if self.mode == RunMode::ForceTier2 && case.native.tier2_entries == 0 {
                    return Err(format!(
                        "{} {:?} passed forced Tier2 without Tier2 entry",
                        case.path, case.variant
                    ));
                }
                if case.native.unexpected_fallbacks != 0 {
                    return Err(format!(
                        "{} {:?} had unexpected fallback",
                        case.path, case.variant
                    ));
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FeatureDisposition {
    Supported,
    Unsupported(Vec<String>),
}

/// Classifies the stable host features needed by focused/replay tests.
/// Full-suite runners additionally load the pinned corpus feature registry;
/// a spelling absent from both sources is a configuration error, never a skip.
pub fn classify_features(features: &[String]) -> Result<FeatureDisposition, String> {
    const SUPPORTED: &[&str] = &[
        "ArrayBuffer",
        "BigInt",
        "DataView",
        "Map",
        "Proxy",
        "Reflect",
        "Set",
        "SharedArrayBuffer",
        "Symbol",
        "TypedArray",
        "WeakMap",
        "WeakRef",
    ];
    const UNSUPPORTED: &[&str] = &[
        "agent",
        "Atomics",
        "IsHTMLDDA",
        "Intl",
        "ShadowRealm",
        "Temporal",
    ];
    let mut unsupported = Vec::new();
    for feature in features {
        if SUPPORTED.contains(&feature.as_str()) {
            continue;
        }
        if UNSUPPORTED.contains(&feature.as_str()) {
            unsupported.push(feature.clone());
            continue;
        }
        return Err(format!("unknown Test262 feature {feature}"));
    }
    if unsupported.is_empty() {
        Ok(FeatureDisposition::Supported)
    } else {
        Ok(FeatureDisposition::Unsupported(unsupported))
    }
}

pub fn classify_features_against(
    features: &[String],
    registry: &str,
) -> Result<FeatureDisposition, String> {
    let known: std::collections::BTreeSet<_> = registry
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect();
    let mut unsupported = Vec::new();
    for feature in features {
        if !known.contains(feature.as_str()) {
            return Err(format!("unknown Test262 feature {feature}"));
        }
        if feature == "IsHTMLDDA"
            || feature == "agent"
            || feature.starts_with("Intl")
            || matches!(feature.as_str(), "ShadowRealm" | "Temporal")
        {
            unsupported.push(feature.clone());
        }
    }
    if unsupported.is_empty() {
        Ok(FeatureDisposition::Supported)
    } else {
        Ok(FeatureDisposition::Unsupported(unsupported))
    }
}

pub fn classify_features_with_config(
    features: &[String],
    registry: &str,
    config: &str,
) -> Result<FeatureDisposition, String> {
    let mut known: std::collections::BTreeSet<_> = registry
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect();
    let mut in_features = false;
    let mut skipped = std::collections::BTreeSet::new();
    for raw in config.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.starts_with('[') {
            in_features = line == "[features]";
            continue;
        }
        if !in_features || line.is_empty() {
            continue;
        }
        let (name, value) = line.split_once('=').unwrap_or((line, ""));
        known.insert(name.trim());
        if value.trim().starts_with("skip") {
            skipped.insert(name.trim());
        }
    }
    let mut unsupported = Vec::new();
    for feature in features {
        if !known.contains(feature.as_str()) {
            return Err(format!("unknown Test262 feature {feature}"));
        }
        if skipped.contains(feature.as_str())
            || matches!(feature.as_str(), "IsHTMLDDA" | "agent" | "cross-realm")
        {
            unsupported.push(feature.clone());
        }
    }
    if unsupported.is_empty() {
        Ok(FeatureDisposition::Supported)
    } else {
        Ok(FeatureDisposition::Unsupported(unsupported))
    }
}

pub fn discover_test262(root: &Path) -> io::Result<Vec<PathBuf>> {
    fn visit(path: &Path, output: &mut Vec<PathBuf>) -> io::Result<()> {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                visit(&path, output)?;
            } else if path.extension().is_some_and(|extension| extension == "js") {
                output.push(path);
            }
        }
        Ok(())
    }
    if !root.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("Test262 root {} is not initialized", root.display()),
        ));
    }
    let mut cases = Vec::new();
    visit(root, &mut cases)?;
    cases.sort();
    if cases.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("zero Test262 cases under {}", root.display()),
        ));
    }
    Ok(cases)
}

/// Produces a deterministic object-graph observation program.
///
/// Getters are not invoked while enumerating descriptors. Object identities
/// are assigned before descending so cycles and aliases remain observable.
pub fn canonical_observation_source(expression: &str) -> String {
    format!(
        r#"
(() => {{
  const seen = new Map(); let nextId = 1;
  const primitive = value => {{
    if (value === undefined) return {{type:'undefined'}};
    if (typeof value === 'number') {{
      if (Number.isNaN(value)) return {{type:'number', value:'NaN'}};
      if (Object.is(value, -0)) return {{type:'number', value:'-0'}};
      if (value === Infinity) return {{type:'number', value:'+Infinity'}};
      if (value === -Infinity) return {{type:'number', value:'-Infinity'}};
    }}
    if (typeof value === 'bigint') return {{type:'bigint', value:String(value)}};
    if (typeof value === 'symbol') return {{type:'symbol', global:Symbol.keyFor(value) ?? null, description:value.description ?? null}};
    return {{type:typeof value, value}};
  }};
  const observe = value => {{
    if ((typeof value !== 'object' || value === null) && typeof value !== 'function') return primitive(value);
    if (seen.has(value)) return {{ref:seen.get(value)}};
    const id = nextId++; seen.set(value,id);
    const descriptors = Object.getOwnPropertyDescriptors(value);
    const properties = Reflect.ownKeys(descriptors).map(key => {{
      const d=descriptors[key]; return {{key:observe(key), enumerable:d.enumerable, configurable:d.configurable,
        writable:'writable' in d ? d.writable : null, value:'value' in d ? observe(d.value) : null,
        get:d.get ? String(d.get.name || '<anonymous>') : null, set:d.set ? String(d.set.name || '<anonymous>') : null}};
    }});
    const base={{id, tag:Object.prototype.toString.call(value), prototype:Object.getPrototypeOf(value)?.constructor?.name ?? null, properties}};
    if (value instanceof Map) base.entries=Array.from(value, pair=>pair.map(observe));
    if (value instanceof Set) base.entries=Array.from(value, observe);
    if (ArrayBuffer.isView(value)) base.bytes=Array.from(new Uint8Array(value.buffer,value.byteOffset,value.byteLength));
    if (value instanceof Error) {{ base.error={{name:value.name,message:value.message,stack:String(value.stack||'').replace(/:\d+:\d+/g,':<line>:<col>')}}; }}
    return base;
  }};
  try {{ return JSON.stringify({{ok:true, value:observe(({expression}))}}); }}
  catch (error) {{ return JSON.stringify({{ok:false, exception:observe(error)}}); }}
}})()
"#
    )
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StructuredProgram {
    seed: u64,
    fuel: u16,
    source: String,
}

impl StructuredProgram {
    pub fn generate(seed: u64, fuel: u16) -> Self {
        let fuel = fuel.clamp(1, 256);
        let mut state = seed;
        let mut next = || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            state
        };
        let bound = (next() % u64::from(fuel.max(2))) + 1;
        let a = next() % 97;
        let b = next() % 31;
        let source = format!(
            r#"(()=>{{
const events=[]; const target={{x:{a},get y(){{events.push('get');return this.x}}}};
const proxy=new Proxy(target,{{get(o,k,r){{events.push('proxy:'+String(k));return Reflect.get(o,k,r)}}}});
let total=0; for(let i=0;i<{bound};i++){{ try{{total=(total+proxy.y+i+{b})|0}}catch(e){{events.push(e.name)}} finally{{total|=0}} }}
const closure=x=>()=>x+total; const array=[target,,proxy];
return {{total,closure:closure(1)(),array,events}};
}})()"#
        );
        Self { seed, fuel, source }
    }
    pub const fn seed(&self) -> u64 {
        self.seed
    }
    pub const fn fuel(&self) -> u16 {
        self.fuel
    }
    pub fn source(&self) -> &str {
        &self.source
    }
    pub fn shrink(&self) -> Self {
        Self::generate(self.seed, (self.fuel / 2).max(1))
    }
}
