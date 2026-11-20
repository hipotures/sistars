# SPEC.md – SI-Stars (Super Intelligence Stars!)

## 1. Vision
**SI-Stars** is a continuous-time 4X space strategy game (with active pause and variable speed `1x–20x`), inspired by *Stars!* (1995).

The project serves two primary goals:
1. **Human vs. AI gameplay** in the browser (high performance via Rust → WASM + WebGL/WebGPU).
2. **AI vs. AI Arena (Policy-driven AI):** The engine must run headless at high throughput (thousands of ticks/games per second) to train and evaluate AI agents. Rather than micromanaging individual units, AI agents will design and optimize high-level **policies** (budget allocation, shipyard doctrines, research priorities, logistics).

## 2. Reference Prototype (`index.html`)
An uncommitted `index.html` file is present in the repository root. It is a **disposable reference demo (v12.0)** created to prototype and balance the core loop. Use it solely as a behavioral reference and **delete it** once the mechanics are implemented in the target architecture.

Core mechanics validated in the prototype:
* **Continuous Time ($\Delta t$):** Fixed simulation timestep; speed scaling via multiple ticks per frame.
* **3-Mineral Economy:** *Ironium* (hulls/armor), *Boranium* (weapons/bombers/defense), *Germanium* (factories/electronics). Finite planetary crust depletion + *Alchemy* (converting surplus energy/resources into missing minerals).
* **Population & Industry:** Logistic population growth, hard planetary building caps (`min(buildings, population * limit)`), and *Empire Sprawl* administrative penalty.
* **Shipyard & 3 Combat Classes:** Cruisers (fast interceptors), Battleships (heavy Ironium armor), Bombers (Boranium, bunker penetration), plus Colonizers and multi-mineral Freighters.
* **Deep Space Combat:** Mid-flight interception, counter-triangle (Cruiser > Bomber > Bunker; Battleship > Cruiser), and salvageable debris fields.
* **Network Logistics:** Natural *Wormholes* and buildable *Stargate* networks.
* **6 Tech Fields (*Stars!* model):** *Energy, Weapons, Propulsion, Construction, Electronics, Biotechnology* with logarithmic progression and multi-field breakthroughs.
* **AI Co-evolution & Monte Carlo Tournaments:** Multi-map win-rate evaluation and alternating best-response evolution of civilization policy vectors (DNA).

## 3. External Data (Initial Setup)
* Fetch technology, tech-field, and ship component definitions from **`sirgwain/craig-stars`** (`https://github.com/sirgwain/craig-stars`, MIT License).
* Store them under `data/craig-stars/` without altering their balance values yet.
* Include full source attribution and MIT license/copyright details in `README.md`.

## 4. Target Stack
* **Simulation Core:** Rust (`crates/engine`), memory-efficient, zero UI dependencies.
* **Browser Bridge:** WebAssembly (`wasm-pack` / `wasm-bindgen`).
* **Frontend:** TypeScript + Vite (`web/`), recreating the full-screen map HUD, planet inspector, event log, and slide-out drawers (Research, Stats/AI, Reports, Settings) from `index.html`.