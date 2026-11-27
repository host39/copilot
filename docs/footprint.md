# Footprint

How big Copilot is, measured at the end of the build (roadmap P6.3) next to the baseline taken when the workspace was first scaffolded (P1.1, `99130cc`) and the numbers the final review measured (P6.1, at `48a103a`). "Lighter than Open WebUI" was an owner requirement, so this page keeps the numbers honest: what grew, what shrank, and how each was measured.

Measured on 2026-11-27 on x86_64 Linux (4 vCPUs) with rustc 1.100.0, Docker 29.9.3, and Postgres 18.1, at the code of `8703919` (the commits after it change only docs and `scripts/footprint.sh`). The P1.1 baseline was a scaffold: health endpoints, the template's middleware, and the adapter crates with their planned dependencies but no code. Growth since then is the product.

## Summary

| Metric | P1.1 baseline | P6.1 review | Now (P6.3) | CI budget |
| --- | --- | --- | --- | --- |
| `server` binary, release profile as CI builds it | 8,395,616 B (8.0 MiB, not stripped) | 30,867,112 B (not stripped) | 22,271,536 B (21.2 MiB, stripped) | 24 MiB |
| `server`, stripped, both rebuilt with rustc 1.100 | 5,952,056 B | | 22,271,536 B (3.7 times) | |
| `migrate` binary, release profile | 6,871,584 B (not stripped) | 7,181,744 B | 5,209,888 B (stripped) | 6 MiB |
| `server` in the image (`dist`: fat LTO, one codegen unit, stripped) | | | 16,120,456 B (15.4 MiB) | |
| Image, compressed layers (pull size) | | 44.5 MB (Debian slim) | 19.0 MB (distroless) | 22 MB |
| Image, unpacked | | 178.6 MB | 49.6 MB | |
| Unique crates, `cargo tree --workspace -e normal` | 208 | 237 | 218 | |
| Unique crates in the server's graph, `-p copilot-api` | 170 | 229 | 210 | |
| Packages in `Cargo.lock` | 295 | 344 | 326 | |
| Web bundle, all files (Rsbuild's total) | 409.2 kB (131.9 kB gzip) | 1042.6 kB (330.8 kB gzip) | 1052.3 kB (334.1 kB gzip) | |
| Web first load (the JS and CSS `index.html` loads) | | 152.8 kB gzip | 152.8 kB gzip, 132.9 kB as served (Brotli) | 170 kB gzip |
| Idle memory (RSS) of the server | | 21.3 MB | 22.0 MB (release), 17.5 MB (image) | |
| Cold start to the first `/health` 200 | | 62 to 75 ms | 55 to 70 ms (release binary), 181 to 203 ms (`docker run` included) | |

CI measured the same commit's code within a few kilobytes (run 46046421711: `server` 22,288,040 bytes, `migrate` 5,222,296, first load 152,756 gzip; run 46046421706: the image at 19,028,337). `scripts/footprint.sh` checks the four budgets in CI on every push (binaries in the Rust job, first load in the lint job, image in `image.yml`), so growth past them fails the build and needs a deliberate change to the script.

## Binaries

- **Release profile** (`make build`, and what CI builds for the smoke test and the budgets): `server` 22,271,536 bytes, `migrate` 5,209,888, `seed` 4,483,280. `[profile.release]` strips symbols since P6.1; without stripping, the same build is 36,856,944 bytes for `server`.
- **Image profile** (`dist`, only the Dockerfile builds it): `server` 16,120,456 bytes, `migrate` 3,526,720.
- **Against the baseline:** the baseline's 8.0 MiB was the default profile without stripping, built with rustc 1.97. Rebuilt today with rustc 1.100 and stripped, the baseline `server` is 5,952,056 bytes, so the stripped server grew 3.7 times (5.7 MiB to 21.2 MiB) while gaining the conversation engine, two provider protocols, the MCP client, the scheduler, files, and lib-rustauth; `migrate` stayed about the same (4,936,440 to 5,209,888 bytes stripped).

## Image

`copilot:p63`, built from the repository's `Dockerfile` (the same stages and pinned digests; the base images came through Google's Docker Hub mirror because Docker Hub rate-limited this host, and the two build stages trusted this host's HTTPS proxy, which leaves the runtime stage unchanged):

| Layer | Unpacked |
| --- | --- |
| Distroless `cc-debian12` base (glibc, libssl, libstdc++, CA roots, tzdata) | 27.9 MB |
| `server` and `migrate` (`dist` profile) | 19.7 MB |
| Web bundle with its `.gz` and `.br` copies | 2.0 MB |
| Total | 49.6 MB |

The compressed layers (what `docker pull` transfers, and what `docker save` writes) are 19.06 MB. Docker reports different numbers for the same image depending on its image store: CI's runner prints 19,028,360 bytes from `docker image inspect` (the compressed size, which the budget checks), while this host's Docker 29 with the containerd store prints 68.7 MB (unpacked and compressed together). The image before P6.1 (Debian slim, unstripped binaries) was 44.5 MB compressed on the runner and 178.6 MB as the reviewer measured it. Building the image took 3.5 minutes here, 3.2 of them in the Rust stage, starting with an empty Cargo cache.

## Dependencies

Counted as the P1.1 baseline counted them: unique `name version` lines of `cargo tree --prefix none --format '{p}'` with the `(*)` and `(proc-macro)` markers removed. The same command run on the baseline and review commits gives their recorded numbers (208 and 237).

| Graph | P1.1 | P6.1 | Now |
| --- | --- | --- | --- |
| Workspace, normal dependencies | 208 | 237 | 218 |
| Workspace without the dev-only `fake-llm` | | | 211 |
| Workspace, normal and build dependencies | 212 | | 224 |
| `copilot-api`, normal dependencies | 170 | 229 | 210 |
| `Cargo.lock` packages | 295 | 344 | 326 |

P6.1 removed `futures-core`, `mime_guess`, and `tokio-stream` from the workspace, `tokio-util` from the providers, reqwest's `multipart` feature, and twenty ICU4X packages (by pinning `idna_adapter` to 1.1). There is no machine learning crate or package, no `aws-lc-rs`, and no OpenSSL; `ring` is the only crate that compiles C code. The web app's runtime dependencies are the same thirteen packages as at P1.1.

## Web Bundle

`pnpm --filter @copilot/web build` (Rsbuild 2.3.5): 1052.3 kB in all, 334.1 kB gzip, most of it in pages loaded on demand. What `index.html` loads before the first paint (React, the router, the shared chunk, the app entry, the stylesheet, and the theme script) is 152,771 bytes with `gzip -9`; the image serves the precompressed copies, 132,899 bytes with Brotli. The first load is the same as at P6.1. Zod's classic API on the first load (about 27 kB gzip) stays for the reasons in [p6.1-client.md](./design/tracks/p6.1-client.md#decisions).

## Memory and Start

The server ran against a migrated throwaway database on the local Postgres, three starts each, with `ENVIRONMENT=production` and `LOG_LEVEL=warn`:

| Build | First `/health` 200 | RSS 2 s after start | RSS after 42 s (a scheduler tick) | Peak RSS | Threads |
| --- | --- | --- | --- | --- | --- |
| Release binary on the host | 55 to 70 ms | 21,444 KiB | 21,516 KiB (22.0 MB) | 29,176 KiB | 5 |
| Image (`docker run`, host network) | 181 to 203 ms | 17,048 KiB | 17,052 KiB (17.5 MB) | 26,568 KiB | 5 |

Both runs set `MALLOC_MMAP_THRESHOLD_` as the image does; the image's server probably uses less memory because its `dist` binary is smaller (fewer code pages resident) and it runs on Debian 12's glibc. The start time of the image includes creating the container. A good part of the start and most of the peak come from lib-rustauth's dummy Argon2 hash (about 25 ms of the start and the 19 MiB peak at P6.1), recorded for lib-rustauth's next version. These are idle numbers: a streaming answer, an upload, or a download adds its own buffers, which the run, upload, and download limits bound ([copilot.md, Limits](./design/copilot.md#limits)).

## How to Measure Again

```bash
cargo build --release -p copilot-api -p copilot-database
bash scripts/footprint.sh binaries target/release
pnpm --filter @copilot/web build && bash scripts/footprint.sh web apps/web/dist
docker build -t copilot:local . && bash scripts/footprint.sh image copilot:local
cargo tree --workspace -e normal --prefix none --format '{p}' | sed -E 's/ \(\*\)$//; s/ \(proc-macro\)//' | sort -u | wc -l
```

For memory, start the server against a migrated database and read `VmRSS` and `VmHWM` from `/proc/<pid>/status` after it has been idle for a scheduler tick (30 seconds).
