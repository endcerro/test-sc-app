# SC Companion

A Star Citizen companion desktop app written in Rust. Early stage: the universe data layer
(location tree, positions, distances, name lookup), an egui GUI with a map and a hauling
contract reader (drop mobiGlas contract screenshots, they are OCR'd and listed) are in place;
the cargo route optimizer comes next.

## Quick start

```sh
cargo run -p sc-companion -- summary            # downloads data on first run, then cached
cargo run -p sc-companion -- tree hurston --depth 1
cargo run -p sc-companion -- find "everus harb0r"
cargo run -p sc-companion -- find "riker memorial spaceport"   # venue alias -> Area18
cargo run -p sc-companion -- dist Lorville "Ruin Station"
cargo run -p sc-companion -- trading pyro
cargo run -p sc-companion                                  # GUI
cargo run -p sc-companion --release --features ocr         # GUI with contract OCR (models downloaded on first use)
```

Data comes from [scunpacked-data](https://github.com/StarCitizenWiki/scunpacked-data) and is
cached in the platform cache directory (`~/.cache/sc-companion/scunpacked` on Linux). If the
network is down, the last cached revision is used.

## Docs

- [`docs/architecture.md`](docs/architecture.md) — crates, model, loading pipeline.
- [`docs/data-sources.md`](docs/data-sources.md) — data source investigation and file formats.
- [`docs/ocr.md`](docs/ocr.md) — contract OCR: layout, preprocessing, engine benchmark, app usage.
- [`CLAUDE.md`](CLAUDE.md) — contributor conventions and commands.

Star Citizen is a trademark of Cloud Imperium Games. This is an unofficial fan project.
