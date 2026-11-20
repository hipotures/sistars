# AGENTS.md – SI-Stars Repository Guidelines (Project SIStars, sistars, SIS)

## Environment & Permissions
* You are running on a dedicated Ubuntu VM with full access to this project workspace.
* You may freely install local project dependencies (`cargo` crates, `wasm-pack`, `npm`/`pnpm` packages).
* **No `sudo` / `apt` privileges:** If a system package is missing (e.g., `pkg-config`, `libssl-dev`, `binaryen`), do not attempt workarounds. Explicitly inform the user with the exact `sudo apt install ...` command required.

## Architecture & Scope
* Follow the simplest modular structure that meets current requirements—avoid speculative abstractions or premature indirection:
  * `crates/engine/` – Deterministic, headless simulation engine in Rust (compiled to native binary for fast AI simulations and to WASM for the browser).
  * `web/` – Visualization and UI layer (TypeScript + Vite + Canvas/WebGL/WebGPU).
  * `data/` – External static data files (JSON).
* Do not maintain monoliths; separate simulation logic, data definitions, and rendering cleanly.
* Do not preserve backward compatibility with the temporary `index.html` prototype. Once its mechanics are migrated, delete it completely.

## External Data & Attribution
* External assets imported from `sirgwain/craig-stars` (MIT License) must be stored in `data/craig-stars/` unmodified at this stage.
* Document the source repository, author, and MIT copyright notice clearly in `README.md`.