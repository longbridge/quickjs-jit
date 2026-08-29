#![allow(clippy::uninlined_format_args)]
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{self, Command},
};

// WASI logic lifted from https://github.com/bytecodealliance/javy/blob/61616e1507d2bf896f46dc8d72687273438b58b2/crates/quickjs-wasm-sys/build.rs#L18

const WASI_SDK_VERSION_MAJOR: usize = 24;
const WASI_SDK_VERSION_MINOR: usize = 0;

#[cfg(feature = "jit-abi")]
#[derive(Debug)]
struct JitOpcode {
    name: String,
    size: u8,
    n_pop: u8,
    n_push: u8,
    format: u8,
}

#[cfg(feature = "jit-abi")]
fn macro_invocations<'a>(source: &'a str, marker: &'a str) -> impl Iterator<Item = &'a str> {
    source.split(marker).skip(1).filter_map(|tail| {
        let tail = tail.strip_prefix('(')?;
        Some(tail.split_once(')')?.0)
    })
}

#[cfg(feature = "jit-abi")]
fn hash_u64(mut hash: u64, mut value: u64) -> u64 {
    for _ in 0..8 {
        hash ^= value & 0xff;
        hash = hash.wrapping_mul(0x100000001b3);
        value >>= 8;
    }
    hash
}

#[cfg(feature = "jit-abi")]
fn generate_jit_opcode_metadata(src_dir: &Path, out_dir: &Path) {
    let expansion_source = out_dir.join("quickjs-jit-opcode-expand.c");
    fs::write(
        &expansion_source,
        r#"
#define FMT(format) QJSJIT_FORMAT(format)
#define DEF(id, size, n_pop, n_push, format) QJSJIT_OPCODE(id, size, n_pop, n_push, format)
#define def(id, size, n_pop, n_push, format) QJSJIT_TEMP_OPCODE(id, size, n_pop, n_push, format)
#include "quickjs-opcode.h"
"#,
    )
    .expect("write QuickJS opcode expansion source");

    let compiler = cc::Build::new().get_compiler();
    let mut command = Command::new(compiler.path());
    command.args(compiler.args());
    if compiler.is_like_msvc() {
        command
            .arg("/nologo")
            .arg("/EP")
            .arg(format!("/I{}", src_dir.display()))
            .arg(&expansion_source);
    } else {
        command
            .arg("-E")
            .arg("-P")
            .arg(format!("-I{}", src_dir.display()))
            .arg(&expansion_source);
    }
    let output = command
        .output()
        .expect("run C preprocessor for QuickJS opcodes");
    if !output.status.success() {
        panic!(
            "QuickJS opcode macro expansion failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let expanded = String::from_utf8(output.stdout).expect("opcode expansion is UTF-8");

    let formats: Vec<String> = macro_invocations(&expanded, "QJSJIT_FORMAT")
        .map(|format| format.trim().to_owned())
        .collect();
    assert!(!formats.is_empty(), "QuickJS exported no operand formats");

    let mut opcodes = Vec::new();
    for invocation in macro_invocations(&expanded, "QJSJIT_OPCODE") {
        let fields: Vec<_> = invocation.split(',').map(str::trim).collect();
        assert_eq!(fields.len(), 5, "malformed opcode expansion: {invocation}");
        let format = formats
            .iter()
            .position(|candidate| candidate == fields[4])
            .unwrap_or_else(|| panic!("unknown opcode format: {}", fields[4]));
        opcodes.push(JitOpcode {
            name: fields[0].to_owned(),
            size: fields[1].parse().expect("opcode size"),
            n_pop: fields[2].parse().expect("opcode pop count"),
            n_push: fields[3].parse().expect("opcode push count"),
            format: format.try_into().expect("operand format fits u8"),
        });
    }
    assert!(!opcodes.is_empty(), "QuickJS exported no opcodes");
    assert!(
        opcodes.len() <= 256,
        "QuickJS opcode IDs must fit in one byte"
    );

    let mut fingerprint = 0xcbf29ce484222325_u64;
    for (opcode, info) in opcodes.iter().enumerate() {
        fingerprint = hash_u64(fingerprint, opcode as u64);
        fingerprint = hash_u64(fingerprint, info.size.into());
        fingerprint = hash_u64(fingerprint, info.n_pop.into());
        fingerprint = hash_u64(fingerprint, info.n_push.into());
        fingerprint = hash_u64(fingerprint, info.format.into());
    }

    fs::write(
        out_dir.join("quickjs-jit-opcodes.generated.h"),
        format!(
            "#define QJSJIT_GENERATED_OPCODE_COUNT {}u\n#define QJSJIT_GENERATED_OPCODE_FINGERPRINT UINT64_C(0x{fingerprint:016x})\n",
            opcodes.len()
        ),
    )
    .expect("write generated QuickJS opcode C metadata");

    let mut constants = String::new();
    let mut rust = format!(
        "pub const QJSJIT_GENERATED_OPCODE_COUNT: usize = {};\n\
         pub const QJSJIT_GENERATED_OPCODE_FINGERPRINT: u64 = 0x{fingerprint:016x};\n\
         pub static QJSJIT_GENERATED_OPCODES: &[JitGeneratedOpcode] = &[\n",
        opcodes.len()
    );
    for (opcode, info) in opcodes.iter().enumerate() {
        rust.push_str(&format!(
            "    JitGeneratedOpcode {{ opcode: {opcode}, size: {}, n_pop: {}, n_push: {}, format: {}, format_name: {:?}, name: {:?} }},\n",
            info.size, info.n_pop, info.n_push, info.format, formats[info.format as usize], info.name
        ));
        constants.push_str(&format!(
            "pub const QJS_JIT_OP_{}: u8 = {opcode};\n",
            info.name.to_ascii_uppercase()
        ));
    }
    rust.push_str("];\n");
    rust.push_str(&constants);
    fs::write(out_dir.join("quickjs-jit-opcodes.rs"), rust)
        .expect("write generated QuickJS opcode Rust metadata");
}

fn download_wasi_sdk() -> PathBuf {
    let mut wasi_sdk_dir: PathBuf = env::var("OUT_DIR").unwrap().into();
    wasi_sdk_dir.push("wasi-sdk");

    fs::create_dir_all(&wasi_sdk_dir).unwrap();

    let major_version = WASI_SDK_VERSION_MAJOR;
    let minor_version = WASI_SDK_VERSION_MINOR;

    let mut archive_path = wasi_sdk_dir.clone();
    archive_path.push(format!("wasi-sdk-{major_version}-{minor_version}.tar.gz"));

    println!("SDK tar: {archive_path:?}");

    // Download archive if necessary
    if !archive_path.try_exists().unwrap() {
        let file_suffix = match (env::consts::OS, env::consts::ARCH) {
            ("linux", "x86") | ("linux", "x86_64") => "x86_64-linux",
            ("linux", "aarch64") => "arm64-linux",
            ("macos", "x86") | ("macos", "x86_64") => "x86_64-macos",
            ("macos", "aarch64") => "arm64-macos",
            ("windows", "x86") | ("windows", "x86_64") => "x86_64-windows",
            ("windows", "aarch64") => "arm64-windows",
            other => panic!("Unsupported platform tuple {:?}", other),
        };

        let uri = format!("https://github.com/WebAssembly/wasi-sdk/releases/download/wasi-sdk-{major_version}/wasi-sdk-{major_version}.{minor_version}-{file_suffix}.tar.gz");

        println!("Downloading WASI SDK archive from {uri} to {archive_path:?}");

        let output = process::Command::new("curl")
            .args([
                "--location",
                "-o",
                archive_path.to_string_lossy().as_ref(),
                uri.as_ref(),
            ])
            .output()
            .expect("failed to download the WASI SDK with curl");
        println!("curl output: {}", String::from_utf8_lossy(&output.stdout));
        println!("curl err: {}", String::from_utf8_lossy(&output.stderr));
        if !output.status.success() {
            panic!(
                "curl WASI SDK failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    let mut test_binary = wasi_sdk_dir.clone();
    test_binary.extend(["bin", "wasm-ld"]);
    // Extract archive if necessary
    if !test_binary.try_exists().unwrap() {
        println!("Extracting WASI SDK archive {archive_path:?}");
        let output = process::Command::new("tar")
            .args([
                "-zxf",
                archive_path.to_string_lossy().as_ref(),
                "--strip-components",
                "1",
            ])
            .current_dir(&wasi_sdk_dir)
            .output()
            .unwrap();
        if !output.status.success() {
            panic!(
                "Unpacking WASI SDK failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    wasi_sdk_dir
}

fn get_wasi_sdk_path() -> PathBuf {
    std::env::var_os("WASI_SDK")
        .map(PathBuf::from)
        .unwrap_or_else(download_wasi_sdk)
}

fn main() {
    #[cfg(feature = "logging")]
    pretty_env_logger::init();

    let features = [
        "bindgen",
        "update-bindings",
        "dump-bytecode",
        "dump-gc",
        "dump-gc-free",
        "dump-free",
        "dump-leaks",
        "dump-mem",
        "dump-objects",
        "dump-atoms",
        "dump-shapes",
        "dump-module-resolve",
        "dump-promise",
        "dump-read-object",
        "disable-assertions",
        "jit-abi",
    ];

    for feature in &features {
        println!("cargo:rerun-if-env-changed={}", feature_to_cargo(feature));
    }
    println!("cargo:rerun-if-env-changed=CARGO_CFG_SANITIZE");

    let src_dir = Path::new("quickjs");

    let out_dir = env::var("OUT_DIR").expect("No OUT_DIR env var is set by cargo");
    let out_dir = Path::new(&out_dir);

    #[cfg(feature = "jit-abi")]
    generate_jit_opcode_metadata(src_dir, out_dir);

    let header_files = [
        "builtin-array-fromasync.h",
        "builtin-iterator-zip-keyed.h",
        "builtin-iterator-zip.h",
        "cutils.h",
        "dtoa.h",
        "libregexp-opcode.h",
        "libregexp.h",
        "libunicode-table.h",
        "libunicode.h",
        "list.h",
        "quickjs-atom.h",
        "quickjs-opcode.h",
        "quickjs-c-atomics.h",
        "quickjs.h",
        "quickjs-jit.h",
    ];

    let source_files = ["libregexp.c", "libunicode.c", "quickjs.c", "dtoa.c"];

    println!("cargo:rerun-if-changed=quickjs.bind.h");
    for file in source_files.iter().chain(header_files.iter()) {
        println!("cargo:rerun-if-changed={}", src_dir.join(file).display());
    }

    let mut defines: Vec<(String, Option<&str>)> = vec![("_GNU_SOURCE".into(), None)];

    #[cfg(feature = "disable-assertions")]
    defines.push(("NDEBUG".into(), None));

    #[cfg(feature = "jit-abi")]
    defines.push(("CONFIG_JIT_ABI".into(), Some("1")));

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap();

    let mut builder = cc::Build::new();
    builder
        .extra_warnings(false)
        .flag_if_supported("-Wno-implicit-const-int-float-conversion")
        //.flag("-Wno-array-bounds")
        //.flag("-Wno-format-truncation")
        ;

    match env::var("CARGO_CFG_SANITIZE").as_deref() {
        Ok("address") => {
            builder
                .flag("-fsanitize=address")
                .flag("-fno-sanitize-recover=all")
                .flag("-fno-omit-frame-pointer");
        }
        Ok("memory") => {
            builder
                .flag("-fsanitize=memory")
                .flag("-fno-sanitize-recover=all")
                .flag("-fno-omit-frame-pointer");
        }
        Ok("thread") => {
            builder
                .flag("-fsanitize=thread")
                .flag("-fno-sanitize-recover=all")
                .flag("-fno-omit-frame-pointer");
        }
        Ok(x) => println!("cargo:warning=Unsupported sanitize_option: '{x}'"),
        _ => {}
    }

    let mut bindgen_cflags = vec![];

    if target_os == "windows" {
        if target_env == "msvc" {
            env::set_var(
                "CFLAGS",
                "/DWIN32_LEAN_AND_MEAN /std:c11 /experimental:c11atomics",
            );
        } else {
            env::set_var("CFLAGS", "-DWIN32_LEAN_AND_MEAN -std=c11");
        }
    }

    if target_os == "wasi" {
        // pretend we're emscripten - there are already ifdefs that match
        // also, wasi doesn't ahve FE_DOWNWARD or FE_UPWARD
        defines.push(("EMSCRIPTEN".into(), Some("1")));
        defines.push(("FE_DOWNWARD".into(), Some("0")));
        defines.push(("FE_UPWARD".into(), Some("0")));
    }

    for file in source_files.iter().chain(header_files.iter()) {
        fs::copy(src_dir.join(file), out_dir.join(file))
            .expect("Unable to copy source; try 'git submodule update --init'");
    }
    fs::copy("quickjs.bind.h", out_dir.join("quickjs.bind.h")).expect("Unable to copy source");

    if target_os == "wasi" && !matches!(env::var("RQUICKJS_SYS_NO_WASI_SDK").as_deref(), Ok("1")) {
        let wasi_sdk_path = get_wasi_sdk_path();
        if !wasi_sdk_path.try_exists().unwrap() {
            panic!(
                "wasi-sdk not installed in specified path of {}",
                wasi_sdk_path.display()
            );
        }
        env::set_var("CC", wasi_sdk_path.join("bin/clang").to_str().unwrap());
        env::set_var("AR", wasi_sdk_path.join("bin/ar").to_str().unwrap());
        let sysroot = format!(
            "--sysroot={}",
            wasi_sdk_path.join("share/wasi-sysroot").display()
        );
        env::set_var("CFLAGS", &sysroot);
        bindgen_cflags.push(sysroot);
    }

    // generating bindings
    bindgen(
        out_dir,
        out_dir.join("quickjs.bind.h"),
        &defines,
        bindgen_cflags,
    );

    for (name, value) in &defines {
        builder.define(name, *value);
    }

    for src in &source_files {
        builder.file(out_dir.join(src));
    }

    builder.compile("libquickjs.a");
}

fn feature_to_cargo(name: impl AsRef<str>) -> String {
    format!("CARGO_FEATURE_{}", feature_to_define(name))
}

fn feature_to_define(name: impl AsRef<str>) -> String {
    name.as_ref().to_uppercase().replace('-', "_")
}

#[cfg(not(feature = "bindgen"))]
fn bindgen<'a, D, H, X, K, V>(out_dir: D, _header_file: H, _defines: X, _add_cflags: Vec<String>)
where
    D: AsRef<Path>,
    H: AsRef<Path>,
    X: IntoIterator<Item = &'a (K, Option<V>)>,
    K: AsRef<str> + 'a,
    V: AsRef<str> + 'a,
{
    let target = env::var("TARGET").unwrap();

    if !Path::new("./")
        .join("src")
        .join("bindings")
        .join(format!("{}.rs", target))
        .canonicalize()
        .map(|x| x.exists())
        .unwrap_or(false)
    {
        println!(
            "cargo:warning=rquickjs probably doesn't ship bindings for platform `{}({})`. try the `bindgen` feature instead.",
            target,
            env::var("BUILD_TARGET").unwrap_or("n/a".into())
        );
    }

    let bindings_file = out_dir.as_ref().join("bindings.rs");

    fs::write(
        bindings_file,
        format!(
            r#"macro_rules! bindings_env {{
                ("TARGET") => {{ "{target}" }};
            }}"#
        ),
    )
    .unwrap();
}

#[cfg(feature = "bindgen")]
fn bindgen<'a, D, H, X, K, V>(out_dir: D, header_file: H, defines: X, add_cflags: Vec<String>)
where
    D: AsRef<Path>,
    H: AsRef<Path>,
    X: IntoIterator<Item = &'a (K, Option<V>)>,
    K: AsRef<str> + 'a,
    V: AsRef<str> + 'a,
{
    let out_dir = out_dir.as_ref();
    let header_file = header_file.as_ref();

    let target = env::var("TARGET").unwrap();
    let host = env::var("HOST").unwrap();

    // When cross-compiling with the `macro` feature, sys also gets built for the host.
    // If LIBCLANG_PATH points at the cross toolchain (e.g. Android NDK), that host build
    // generates mismatched bindings, so reuse the bundled binding for the host instead.
    // `update-bindings` still regenerates.
    if target == host
        && env::var("CARGO_FEATURE_UPDATE_BINDINGS").is_err()
        && env::var("CARGO_FEATURE_JIT_ABI").is_err()
    {
        let bundled = Path::new("src")
            .join("bindings")
            .join(format!("{}.rs", target));
        if bundled.exists() {
            println!(
                "cargo:warning=using bundled bindings for host target `{}` instead of running bindgen (enable the `update-bindings` feature to regenerate)",
                target
            );
            fs::copy(&bundled, out_dir.join("bindings.rs"))
                .expect("Unable to copy bundled bindings");
            return;
        }
    }

    let mut cflags = add_cflags;

    //format!("-I{}", out_dir.parent().display()),

    for (name, value) in defines {
        cflags.push(if let Some(value) = value {
            format!("-D{}={}", name.as_ref(), value.as_ref())
        } else {
            format!("-D{}", name.as_ref())
        });
    }

    let mut builder = bindgen_rs::Builder::default()
        .use_core()
        .detect_include_paths(true)
        .clang_arg("-xc")
        .clang_arg("-v")
        .clang_args(cflags)
        .size_t_is_usize(false)
        .header(header_file.display().to_string())
        .allowlist_type("JS.*")
        .allowlist_function("js.*")
        .allowlist_function("JS.*")
        .allowlist_function("__JS.*")
        .allowlist_var("JS.*")
        .allowlist_var("QJSJIT.*")
        .opaque_type("FILE")
        .blocklist_type("FILE")
        .blocklist_function("JS_DumpMemoryUsage");

    if env::var("CARGO_CFG_TARGET_OS").unwrap() == "wasi" {
        builder = builder.clang_arg("-fvisibility=default");
    }

    let bindings = builder.generate().expect("Unable to generate bindings");

    let bindings_file = out_dir.join("bindings.rs");

    bindings
        .write_to_file(&bindings_file)
        .expect("Couldn't write bindings");

    // Special case to support bundled bindings
    if env::var("CARGO_FEATURE_UPDATE_BINDINGS").is_ok() {
        let dest_dir = Path::new("src").join("bindings");
        fs::create_dir_all(&dest_dir).unwrap();

        let dest_file = format!("{}.rs", env::var("TARGET").unwrap());
        fs::copy(&bindings_file, dest_dir.join(dest_file)).unwrap();
    }
}
