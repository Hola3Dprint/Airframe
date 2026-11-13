# Engineering model

## Steady operating point (desktop default)

Each room and junction satisfies ΣQin − ΣQout = 0. A prescribed-flow fan uses the entered airflow directly, including zero for off. A pressure fan uses ΔP_fan = P_shutoff s² − k Q hypot(Q, 10⁻⁵ m³/s) at the saved speed fraction s. The slider is labeled with the nominal free airflow s√(P_shutoff/k). The actual airflow comes from the intersection with the network load. Older projects default to full speed. Room volume, initial pressure, fan time constants, and scheduled events do not enter this calculation. Each room or junction needs a pressure-dependent path to a fixed-pressure boundary because prescribed flow alone cannot set room pressure.

The circuit view keeps a shunt capacitor on each room, C = V/P_abs, and a series inductor on each duct or riser, L = ρL_duct/A, alongside its nonlinear resistor. At a constant operating point C dP/dt = 0 and L dQ/dt = 0, so capacitors draw no airflow and inductors add no pressure drop. The retained transient solver integrates room capacitance; dynamic duct inertance is represented in the circuit but is not integrated by that solver.

The solver uses damped Newton iterations with dense LU. It checks flow balance and prescribed flow equations to 10⁻⁹ m³/s, and pressure equations to 10⁻⁶ Pa. It reports a diagnostic if the pressure reference is missing or the equations fail to converge. Opening losses have the same small zero-flow regularization as the transient solver. Negative branch flow reverses the in/out accounting in the report.

The desktop shows one solved operating point and can scrub through the optional fan startup simulation. The following transient equations apply to the startup view and the headless transient example.

Airframe models lumped, fixed-volume pressure zones and quasi-steady branches in SI. The display may convert volumetric flow to m³/h or CFM. Air pressure results are gauge pressure relative to the specified absolute reference. Default reference properties are 101,325 Pa, 1.204 kg/m³, and 1.81 × 10⁻⁵ Pa·s. Duct roughness defaults to 0.09 mm.

A branch has one orientation from its first pressure node to its second. Each branch adds −Q to the first node's balance and +Q to the second. Negative flow is allowed. Network connections are UUIDs; drawing lines, their intersections, and visual offsets have no effect on connectivity or length.

| Element | Relation |
| --- | --- |
| Room | C dP/dt = ΣQ, where C = V/P_abs at the reference pressure. This is a small pressure, isothermal approximation with fixed reference density. |
| Junction | 0 = ΣQ. There is no artificial room volume. |
| Boundary | P = prescribed gauge pressure. |
| Opening | ΔP ≈ k Q abs(Q), with k in Pa·s²/m⁶. Integration uses k Q hypot(Q, 10⁻⁵ m³/s) for a finite slope at zero flow. At 600 m³/h the relative correction is below 2 × 10⁻⁹. |
| Duct | Q = A v, Re = ρ abs(v) D_h/μ, ΔP = (f L/D_h + ΣK)ρv abs(v)/2. D_h = D for round ducts and 2wh/(w+h) for rectangular ducts. |
| Flow fan | τ dQ/dt = u Q_rated − Q. The prescribed flow fraction is plotted as a response, not as measured or calculated RPM. |
| Pressure fan | τ ds/dt = u − s; P_from − P_to + P_shutoff s² = k Q hypot(Q, 10⁻⁵ m³/s). This simple signed curve permits reverse flow with the fan off. |

The duct calculation uses Darcy f = 64/Re for laminar flow (hydraulic-diameter approximation for rectangles), Swamee–Jain for turbulent flow, and blends the losses from Re 2300 to 4000. Fittings are included through the summed minor-loss coefficient. The inspector can estimate inertance L_f = ρL/A; the integrator currently treats ducts as quasi-steady.

A duct or riser can also specify an additional pressure loss ΔP_ref at a positive reference airflow Q_ref. The solver adds ΔP_ref × (Q/Q_ref) × abs(Q/Q_ref) to the calculated loss. It vanishes at zero airflow and opposes flow in either direction. Saved projects without this input default to zero additional loss.

## Integration and verification

[Diffsol](https://docs.rs/diffsol/0.19.0/diffsol/) 0.19.0 provides adaptive BDF integration, a consistent initial state for a singular mass matrix, and dense output. Airframe supplies the equations, diagonal mass matrix, and Jacobian-vector product, then uses dense nalgebra LU. The matrix contains room capacitances, fan time constants, and exact zero rows for algebraic equations. Petgraph checks whether every junction has a pressure-dependent path to a room or fixed boundary. The solver restarts at each scheduled command change with the prior physical state, so no adaptive step crosses a fan event. Fans start at rest; initial passive flows are calculated consistently with initial pressures.

[ode_solvers](https://docs.rs/ode_solvers/) was considered for its straightforward adaptive explicit Dormand–Prince method. An explicit ODE solver would require eliminating the junction constraints separately and can take very small steps near zero flow. [russell_ode](https://docs.rs/russell_ode/) also offers an implicit Radau method for index-1 DAEs; Diffsol provides the needed mass-matrix support, exact stop times, interpolation, and explicit error returns in one library.

Relative tolerance is 10⁻⁷; absolute tolerances are 10⁻⁶ Pa for pressure and 10⁻⁹ for flow and speed states. Output includes a regular plotting grid and the exact event times. A junction imbalance larger than 10⁻⁶ m³/s causes a diagnostic error. The reference test checks the analytic fan curve to 2 × 10⁻⁷ m³/s and compares pressure at six times against a separate scalar RK4 integration with a 0.1 ms step to 0.001 Pa.

For the single-room example, Q_rated = 600/3600 = 1/6 m³/s, C = 100/101325 ≈ 9.869 × 10⁻⁴ m³/Pa, and k = 50/(1/6)² = 1800 Pa·s²/m⁶. The room starts at 0 Pa, the fan ramps toward rated flow at t = 0 s, and the off command at 60 s ramps the flow toward zero.

The run captures an immutable engineering network and scenario. Layout edits can leave results usable; changed physical inputs or events require a rerun. Each CSV column identifies its source by name and UUID.

## Current scope

The app supports multiple rooms, junctions, parallel and series paths, reversible flow, several fans and scenarios, and interconnected floors. A riser is a duct between pressure nodes with an explicit length and endpoint elevations. Elevations are for display; hydrostatic stack pressure, temperature differences, and wind pressure are not computed automatically. There is no CFD, dynamic duct inertia, manufacturer fan curve import, backdraft valve, variable damper, or full drawing digitization. The circuit is a live analogy, not a SPICE export. The compact editor uses a single optional bend per branch and a one-branch elevation profile.

The dense solver is intended for small networks; the 300-node / 500-branch goal has not been benchmarked. Larger networks may need sparse linear algebra. A warning is shown if computed gauge pressures exceed 5% of the absolute reference. These models are suitable for development and comparison; they have not been independently validated for facility safety or regulatory decisions.

Next priorities: collect measured room geometry and transfer-path pressure losses for the pilot, calibrate against measured operating conditions, then add sparse solves and optional branch inertance.
