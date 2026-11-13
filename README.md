# Airframe · Ventilation Workbench

Airframe is a native Rust app for drawing ventilation networks and solving one steady airflow operating point. Set the supply, exhaust, and transfer fan flows. The solver calculates room gauge pressures, branch flows, and the airflow balance at each room.

## Run on Windows

Download **Airframe-Setup-x64.exe** from [the latest release](https://github.com/Hola3Dprint/Airframe/releases/latest). Double-click it to install Airframe and open the app. You can also launch Airframe from the Start menu or desktop.

The [quick start](docs/quick-start.md) shows where to open the example and solve it.

The installed app checks GitHub for updates at startup and every six hours while it is open. Open **Settings → Airframe updates** to see the status or check manually. Select **Update to v…** to download and install an available release. Airframe shows progress and reopens with your work.

To build from source, install [stable Rust](https://rustup.rs/) and the Visual Studio C++ build tools with a Windows SDK, then run this from the Program folder:

```powershell
cargo run --release --locked -p app
```

If Cargo isn't on your PowerShell path, run `$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"` first. The build creates `target/release/airframe.exe`.

Run `./installer/build.ps1` with [NSIS](https://nsis.sourceforge.io/Download) installed to build the Windows installer locally. To publish a future update, bump the workspace version in `Cargo.toml` and push a matching tag such as `v0.1.1`. The release workflow builds and uploads the installer to GitHub Releases.

## Use the editor

| Task | How |
| --- | --- |
| Place a node | Drag a room, fixed-pressure boundary, Atmosphere, or junction from the library into the canvas. Atmosphere holds 0 Pa gauge and appears as ground in Circuit. |
| Connect boxes | Drag from a dot on a source box to any part of the destination box. This adds a duct. Choose a fan or opening first to connect with that component instead. You can also click the source and destination after choosing a branch type. |
| Accept or dismiss a suggestion | Click the **+** on a dashed line to add a duct. Right-click the **+** to stop suggesting that pair. |
| Insert in series | Drop a library component onto an editable wire. Branch components use small connection dots. Deleting an inserted component reconnects the surviving sections. Inserting into a fan branch adds a downstream opening with a default loss; review it before solving. |
| Edit multiple components | Shift-click or enable **Multi-select** to select more than one component. Shared fields appear in the inspector. Type a value into a **Mixed** field and press Enter, or change a common value directly. |
| Select by rectangle | Left-drag from empty canvas space to select the node and branch cards inside. Hold Shift or turn on **Multi-select** to add to the selection. |
| Work across floors | Use **Split levels** for two independently panned and zoomed panes. Click a pane to edit it and drag the vertical divider to resize it. Right-click a floor in the library to delete it; Undo restores it. |
| Place a vertical riser | Drop a riser onto blank canvas, a node, or an existing wire. It starts with a same-level rise. Set **Destination floor** in the source landing inspector when it should land on another level. |
| Model air paths | Use **Door**, **Leakage**, **Transfer air**, or **Supply air** from the library. Edit the opening area, discharge coefficient, and open fraction where applicable. |
| Connect rooms with a door | Drag **Door** onto the first room, then click the second room. You can also select **Door** and click both rooms. The door itself is the airflow path. |
| Add duct pressure loss | Select a duct or riser and set **Additional loss** in Pa and **At airflow** in the current airflow units. The extra loss adds to friction and fittings and scales with airflow squared. |
| Reuse a node | Choose **Custom nodes → + New**, or **Save as custom node** in the inspector. Templates live in the Program folder under `projects/local/library` and are available across projects. |
| Solve and export | Click **Solve airflow**, then **Results → Export Excel** for editable inputs and live formulas for the steady equations. The workbook opens at the Airframe solution. After changing inputs, use Excel Solver to rebalance a small network or re-solve in Airframe. CSV is also available. |
| Adjust fan airflow | Use the airflow slider and switch between **Use fan curve** and **Use fixed airflow**. In curve mode the slider sets speed through the fan’s free-air rating. In the inspector, expand **Custom fan curve** and paste rated airflow and pressure pairs. |
| Review startup | Choose **Fan startup → Run startup** below the canvas or in **Results**. Scrub or play the timeline to inspect flows and pressures from the transient run. |
| Navigate and arrange | Scroll to zoom up to 1000%. Right-drag or middle-drag to pan. **Arrange** spaces connected nodes. With a floor plan shown it opens the circuit layout. **Circuit → Full network** draws supply-to-extract runs on horizontal rows with vertical links for doors and risers. Use **Arrange** to restore the automatic circuit layout or **Room focus** to see a room and its direct connections. |
| Inspect a circuit symbol | Click a resistor for its signed pressure-loss curve, an inductor for its inertance, or another symbol for its own value. A solved circuit shows the resistor's operating point. Click **Show component properties** to edit the underlying branch or room. |
| Import a drawing | Use **Layers → Import floor plan image** and pick a PNG or JPEG. The project references the image file; keep it available alongside the project when sharing. |
| Save and recover | Use **File → Save project** or **Save as**. Airframe records unsaved edits in a separate recovery copy about every five seconds. After a crash, choose **Recover** at startup or use **File → Recover unsaved work…**. |

Airframe starts with a blank workspace. Open the single-room example from **File** to see **−50 Pa** at **600 m³/h** (about **353 CFM**). The synthetic two-floor project is also available under **File**.

## Repository

- `crates/airflow`: GUI-free physical model, steady and transient solvers, JSON, CSV, and Excel export.
- `crates/app`: native window, canvas, inspector, and flow report.
- `examples`: synthetic JSON projects. Files ending in `.airflow.local.json` stay outside Git.
- `docs/engineering.md`: equations, solver tolerances, and limitations.

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo run --release --locked -p airflow --example solve_airflow -- examples/single-room.airflow.json
```

The headless example accepts an optional second path ending in `.csv` or `.xlsx`. The separate `simulate_project` example runs the saved transient schedule.
