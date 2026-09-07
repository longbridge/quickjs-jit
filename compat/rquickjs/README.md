# rquickjs compatibility facade

This unpublished package reexports `quickjs-jit` under the package name
`rquickjs`. Dependencies such as LLRT can keep their existing `rquickjs`
dependency while sharing the application's QuickJS types and VM. The facade
forwards features and performs no runtime conversion.

For an application next to a local quickjs-jit checkout, configure its workspace
root as follows:

```toml
[dependencies]
rquickjs = { package = "quickjs-jit", path = "../quickjs-jit" }
rquickjs-jit = { package = "quickjs-jit-runtime", path = "../quickjs-jit/jit", features = ["compiler"] }

[patch.crates-io]
rquickjs = { path = "../quickjs-jit/compat/rquickjs" }
```

Cargo only applies patches declared by the consuming workspace root; declaring
this patch inside a library dependency does not redirect that library's consumers.
The facade satisfies compatible `rquickjs` 0.12 requirements, not older binding
versions.

For a Git dependency, point all three entries to the same repository URL and
full commit revision containing this package. Cargo locates the named packages
within the repository. Keep the bindings, runtime, and facade on that same
source so they resolve to one `quickjs-jit-core` and `quickjs-jit-sys` instance.
The `rquickjs` package name belongs to upstream on crates.io, so this facade has
`publish = false` and is consumed through Git or a local path.
