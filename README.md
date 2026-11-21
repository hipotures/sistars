# SI-Stars (Super Intelligence Stars!)

**SI-Stars** is a continuous-time 4X space strategy game (with active pause and variable speed `1x–20x`), inspired by *Stars!* (1995). It is designed both for browser-based gameplay (Rust → WebAssembly + TypeScript/Vite) and high-throughput headless AI-vs-AI policy simulation.

## Repository Structure

```text
sistars/
├── crates/
│   └── engine/           # Deterministic headless Rust simulation engine + WASM bindings
│       ├── src/
│       │   ├── lib.rs    # Crate entry point
│       │   ├── config.rs # Simulation config, 6 tech fields, breakthroughs, ship designs
│       │   ├── types.rs  # Core data structures, AI doctrines, snapshots
│       │   ├── sim.rs    # Deterministic simulation loop (SoA planets, fleets, combat)
│       │   ├── ai.rs     # Monte Carlo tournaments, co-evolution, auto-balancing
│       │   ├── wasm.rs   # wasm-bindgen browser bridge (wasm32 only)
│       │   └── bin/
│       │       └── headless.rs # Native headless benchmark & tournament CLI
│       └── Cargo.toml
├── web/                  # Browser frontend (TypeScript + Vite + Canvas)
│   ├── src/
│   │   ├── main.ts       # App bootstrap, WASM integration, animation loop
│   │   ├── types.ts      # TypeScript interfaces matching engine snapshots
│   │   ├── renderer.ts   # Galaxy map Canvas renderer & visual FX
│   │   ├── ui.ts         # HUD, Planet Inspector, Event Log, and 4 slide-out drawers
│   │   └── style.css     # Dark tactical HUD styles
│   ├── index.html
│   ├── package.json
│   ├── tsconfig.json
│   └── vite.config.ts
├── data/
│   └── craig-stars/      # Unmodified upstream tech & rule definitions from craig-stars
├── AGENTS.md             # Repository engineering guidelines
├── SPEC.md               # Project specification
└── LICENSE               # MIT License
```

## Prerequisites

- **Rust toolchain:** `rustc`, `cargo`, `rustup` with target `wasm32-unknown-unknown`
- **WASM build tool:** `wasm-pack` (`cargo install wasm-pack --locked`) and `binaryen` (`wasm-opt`)
- **Frontend toolchain:** `node` (v20+) and `npm`

On Ubuntu:
```bash
sudo apt install -y pkg-config libssl-dev npm binaryen
rustup target add wasm32-unknown-unknown
cargo install wasm-pack --locked
```

## Running & Building

### 1. Native Rust Engine (Tests & Headless Benchmark)
```bash
cargo test
cargo run --release --bin headless
```

### 2. WebAssembly + Frontend (Browser)
```bash
# Build WASM package into web/src/wasm
wasm-pack build crates/engine --target web --out-dir ../../web/src/wasm

# Install frontend dependencies and start Vite dev server
cd web
npm install
npm run dev
```

Or build the full production bundle:
```bash
cd web
npm run build
```

## External Data & Attribution

### 1. Game Data (`data/craig-stars/`)
Static game data files stored unmodified under `data/craig-stars/` (`techs.json`, `rules.json`, and `LICENSE`) are imported from the **[craig-stars](https://github.com/sirgwain/craig-stars)** project:

- **Source Repository:** <https://github.com/sirgwain/craig-stars>
- **Upstream Files:** `frontend/src/lib/ssr/techs.json`, `frontend/src/lib/ssr/rules.json` (commit `943ffaea1dd2455180e0054efd420f4c4ff00b85`)
- **Author:** `sirgwain`
- **License:** MIT License

### 2. 3D Spaceship Models (`web/public/models/ships/`)
The low-poly 3D ship hulls (`battleship.glb`, `cruiser.glb`, `bomber.glb`, `freighter.glb`, `colony.glb`) are derived from the **Ultimate Spaceship Pack** by **Quaternius**:

- **Author:** Quaternius (<https://quaternius.com>)
- **License:** CC0 1.0 Universal (Public Domain Dedication) — see `web/public/models/ships/LICENSE-Quaternius-CC0.txt`

```text
MIT License

Copyright (c) 2022 sirgwain

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## License

SI-Stars is licensed under the MIT License. See [LICENSE](LICENSE) for details.
