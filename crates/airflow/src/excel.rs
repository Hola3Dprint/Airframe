//! Recalculable Excel export of the steady network equations.
use crate::{model::*, physics::*, solver::steady::OperatingPoint};
use rust_xlsxwriter::{Color, Format, Formula, Workbook, Worksheet, XlsxError};
use std::{collections::HashMap, path::Path};

const FLOW_SCALE: f64 = 0.001;
const APERTURE_MIN: f64 = 1e-6;
const SPEED_MIN: f64 = 1e-8;

struct Styles {
    heading: Format,
    input: Format,
    calc: Format,
}

impl Styles {
    fn new() -> Self {
        Self {
            heading: Format::new()
                .set_bold()
                .set_font_color(Color::White)
                .set_background_color(0x18314C),
            input: Format::new()
                .set_font_color(0x165B9C)
                .set_background_color(0xEFF7FF)
                .set_num_format("0.000000E+00"),
            calc: Format::new().set_num_format("0.000000E+00"),
        }
    }

    fn headers(&self, ws: &mut Worksheet, names: &[&str]) -> Result<(), XlsxError> {
        ws.set_freeze_panes(1, 1)?;
        for (col, name) in names.iter().enumerate() {
            ws.write_string_with_format(0, col as u16, *name, &self.heading)?;
            ws.set_column_width(col as u16, (name.len() as f64 + 3.0).clamp(14.0, 28.0))?;
        }
        Ok(())
    }

    fn input(&self, ws: &mut Worksheet, row: u32, col: u16, value: f64) -> Result<(), XlsxError> {
        ws.write_number_with_format(row, col, value, &self.input)?;
        Ok(())
    }

    fn cell(
        &self,
        ws: &mut Worksheet,
        row: u32,
        col: u16,
        expr: String,
        value: f64,
    ) -> Result<(), XlsxError> {
        ws.write_formula_with_format(
            row,
            col,
            Formula::new(expr).set_result(value.to_string()),
            &self.calc,
        )?;
        Ok(())
    }
}

/// Export the supplied operating point, the editable inputs, and the live equations.
pub fn export(path: &Path, name: &str, point: &OperatingPoint) -> Result<(), XlsxError> {
    let mut book = Workbook::new();
    let s = Styles::new();
    let net = &point.snapshot;
    let n_last = net.nodes.len() + 1;
    let b_last = net.branches.len() + 1;
    let mut variable = 2;
    let mut node_cells = Vec::new();
    let mut branch_cells = Vec::new();
    let mut curve_ranges = Vec::new();
    let mut sum = 0.0;
    let mut max_pressure_error: f64 = 0.0;
    let mut max_fixed_error: f64 = 0.0;
    book.add_worksheet().set_name("Overview")?;
    {
        let ws = book.add_worksheet().set_name("Inputs")?;
        s.headers(
            ws,
            &[
                "Air / solver constant",
                "Value",
                "",
                "Solver variable",
                "Trial value",
            ],
        )?;
        ws.set_column_width(0, 36)?;
        ws.set_column_width(3, 40)?;
        for (i, (label, value)) in [
            ("Absolute reference pressure · Pa", net.air.absolute_pa),
            ("Air density · kg/m³", net.air.density),
            ("Dynamic viscosity · Pa·s", net.air.viscosity),
            ("Low-flow smoothing · m³/s", LOW_FLOW),
            ("Opening fraction floor", APERTURE_MIN),
            ("Fan stopped speed threshold", SPEED_MIN),
            ("Flow error scaling · m³/s", FLOW_SCALE),
            ("Pressure error scaling · Pa", 1.0),
            ("CFM to m³/s", CFM),
        ]
        .into_iter()
        .enumerate()
        {
            let r = i as u32 + 1;
            ws.write_string(r, 0, label)?;
            s.input(ws, r, 1, value)?;
        }
        for (i, node) in net.nodes.iter().enumerate() {
            if node.kind.fixed_gauge_pa().is_some() {
                node_cells.push(None);
            } else {
                node_cells.push(Some(variable));
                ws.write_string(variable - 1, 3, format!("{} · Pa", node.name))?;
                s.input(ws, variable - 1, 4, point.pressures[i])?;
                variable += 1;
            }
        }
        for (i, b) in net.branches.iter().enumerate() {
            if matches!(b.kind, BranchKind::FlowFan { .. }) {
                branch_cells.push(None);
            } else {
                branch_cells.push(Some(variable));
                ws.write_string(variable - 1, 3, format!("{} · m³/s", b.name))?;
                s.input(ws, variable - 1, 4, point.flows[i])?;
                variable += 1;
            }
        }
    }
    {
        let ws = book.add_worksheet().set_name("Nodes")?;
        s.headers(
            ws,
            &[
                "Node",
                "Type",
                "Level",
                "Floor elev. · m",
                "Volume · m³",
                "Initial gauge · Pa",
                "Fixed gauge · Pa",
                "Gauge · Pa",
                "Capacitance · m³/Pa",
                "In · m³/s",
                "Out · m³/s",
                "Net · m³/s",
                "Balance error · m³/s",
                "Scaled error²",
                "Node ID",
            ],
        )?;
        ws.set_column_width(0, 32)?;
        ws.set_column_width(14, 38)?;
        for (i, node) in net.nodes.iter().enumerate() {
            let row = i as u32 + 1;
            let r = row + 1;
            ws.write_string(row, 0, &node.name)?;
            ws.write_string(row, 1, node.kind.label())?;
            if let Some(floor) = net.floor(node.floor) {
                ws.write_string(row, 2, &floor.name)?;
                ws.write_number(row, 3, floor.elevation_m)?;
            }
            let p = point.pressures[i];
            let mut cap = 0.0;
            match node.kind {
                NodeKind::Room {
                    volume_m3,
                    initial_pa,
                } => {
                    s.input(ws, row, 4, volume_m3)?;
                    s.input(ws, row, 5, initial_pa)?;
                    cap = capacitance(volume_m3, net.air);
                }
                NodeKind::Boundary { gauge_pa } => s.input(ws, row, 6, gauge_pa)?,
                NodeKind::Atmosphere => s.cell(ws, row, 6, "0".into(), 0.0)?,
                NodeKind::Junction => {}
            }
            let trial = node_cells[i].map_or_else(|| format!("G{r}"), |v| format!("Inputs!E{v}"));
            s.cell(ws, row, 7, trial, p)?;
            s.cell(ws, row, 8, format!("E{r}/Inputs!$B$2"), cap)?;
            let balance = point.balances[i];
            let branch_sum = |id: &str, q: &str| {
                if b_last == 1 {
                    "0".to_owned()
                } else {
                    format!(
                        "SUMIF(Branches!${id}$2:${id}${b_last},O{r},Branches!${q}$2:${q}${b_last})"
                    )
                }
            };
            s.cell(
                ws,
                row,
                9,
                format!("{}+{}", branch_sum("AG", "AL"), branch_sum("AF", "AM")),
                balance.inflow,
            )?;
            s.cell(
                ws,
                row,
                10,
                format!("{}+{}", branch_sum("AF", "AL"), branch_sum("AG", "AM")),
                balance.outflow,
            )?;
            s.cell(ws, row, 11, format!("J{r}-K{r}"), balance.net())?;
            let residual = if node_cells[i].is_none() {
                0.0
            } else {
                balance.net()
            };
            s.cell(
                ws,
                row,
                12,
                format!("IF(OR(B{r}=\"Boundary\",B{r}=\"Atmosphere\"),0,L{r})"),
                residual,
            )?;
            let squared = (residual / FLOW_SCALE).powi(2);
            s.cell(ws, row, 13, format!("(M{r}/Inputs!$B$8)^2"), squared)?;
            sum += squared;
            ws.write_string(row, 14, node.id.to_string())?;
        }
    }
    {
        let ws = book.add_worksheet().set_name("Fan curves")?;
        s.headers(ws, &["Fan", "Rated flow · m³/s", "Rated rise · Pa"])?;
        ws.set_column_width(0, 36)?;
        let mut row = 1;
        for b in &net.branches {
            if matches!(b.kind, BranchKind::PressureFan { .. }) && !b.custom_fan_curve.is_empty() {
                let first = row + 1;
                for [q, p] in &b.custom_fan_curve {
                    ws.write_string(row, 0, &b.name)?;
                    s.input(ws, row, 1, *q)?;
                    s.input(ws, row, 2, *p)?;
                    row += 1;
                }
                curve_ranges.push(Some((first, row)));
            } else {
                curve_ranges.push(None);
            }
        }
    }
    {
        let ws = book.add_worksheet().set_name("Branches")?;
        s.headers(
            ws,
            &[
                "Branch",
                "Type",
                "System",
                "From",
                "To",
                "Flow · m³/s",
                "Flow · CFM",
                "Pfrom − Pto · Pa",
                "Passive loss · Pa",
                "Fan rise · Pa",
                "Pressure error · Pa",
                "Flow error · m³/s",
                "Scaled error²",
                "Length · m",
                "Section",
                "Diameter · m",
                "Width · m",
                "Height · m",
                "Roughness · m",
                "Minor K",
                "Opening / fan k",
                "Free area · m²",
                "Discharge Cd",
                "Open fraction",
                "Fixed flow · m³/s",
                "Shutoff · Pa",
                "Speed fraction",
                "Fan response · s",
                "Start elev. · m",
                "End elev. · m",
                "Branch ID",
                "From ID",
                "To ID",
                "Area · m²",
                "Hydraulic D · m",
                "Inertance · Pa·s²/m³",
                "Velocity · m/s",
                "Forward · m³/s",
                "Reverse · m³/s",
                "Reynolds number",
                "Signed dynamic · Pa",
                "Laminar loss · Pa",
                "Turbulent friction f",
                "Turbulent loss · Pa",
                "Turbulent blend",
                "Effective opening k",
                "Smoothed Q term",
                "Custom rated Q · m³/s",
                "Curve segment",
                "Custom reverse drag k",
            ],
        )?;
        for c in [0, 3, 4, 30, 31, 32] {
            ws.set_column_width(c, if c < 5 { 32 } else { 38 })?;
        }
        let index: HashMap<_, _> = net
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id, i))
            .collect();
        for (i, b) in net.branches.iter().enumerate() {
            let row = i as u32 + 1;
            let r = row + 1;
            let (a, z) = (index[&b.from], index[&b.to]);
            let (ar, zr) = (a + 2, z + 2);
            let q = point.flows[i];
            let dp = point.pressures[a] - point.pressures[z];
            let (mut loss, mut rise, mut flow_error) = (0.0, 0.0, 0.0);
            for (c, value) in [
                (0, b.name.clone()),
                (1, b.kind.label().into()),
                (2, b.system.label().into()),
                (3, net.nodes[a].name.clone()),
                (4, net.nodes[z].name.clone()),
                (30, b.id.to_string()),
                (31, b.from.to_string()),
                (32, b.to.to_string()),
            ] {
                ws.write_string(row, c, value)?;
            }
            let q_ref = branch_cells[i].map_or_else(|| format!("Y{r}"), |v| format!("Inputs!E{v}"));
            s.cell(ws, row, 5, q_ref, q)?;
            s.cell(ws, row, 6, format!("F{r}/Inputs!$B$10"), q / CFM)?;
            s.cell(ws, row, 7, format!("Nodes!H{ar}-Nodes!H{zr}"), dp)?;
            s.cell(ws, row, 37, format!("MAX(F{r},0)"), q.max(0.0))?;
            s.cell(ws, row, 38, format!("MAX(-F{r},0)"), (-q).max(0.0))?;
            s.cell(
                ws,
                row,
                46,
                format!("F{r}*SQRT(F{r}^2+Inputs!$B$5^2)"),
                q * q.hypot(LOW_FLOW),
            )?;
            ws.write_number(row, 28, b.start_elevation_m)?;
            ws.write_number(row, 29, b.end_elevation_m)?;
            if let Some(tau) = b.kind.tau() {
                s.input(ws, row, 27, tau)?;
            }
            match b.kind {
                BranchKind::Opening { k } => {
                    s.input(ws, row, 20, k)?;
                    if let Some(a) = b.aperture {
                        s.input(ws, row, 21, a.area_m2)?;
                        s.input(ws, row, 22, a.discharge)?;
                        s.input(ws, row, 23, a.open_fraction)?;
                    }
                    let k_eff = b.aperture.map_or(k, |a| a.resistance(net.air.density));
                    s.cell(ws, row, 45, format!("IF(ISNUMBER(V{r}),Inputs!$B$3/(2*(V{r}*W{r}*MAX(X{r},Inputs!$B$6))^2),U{r})"), k_eff)?;
                    loss = opening_loss(k_eff, q);
                    s.cell(ws, row, 8, format!("AT{r}*AU{r}"), loss)?;
                }
                BranchKind::Duct { duct, .. } => {
                    loss = write_duct(ws, &s, row, duct, net.air, q)?;
                }
                BranchKind::FlowFan { design_m3_s, .. } => {
                    s.input(ws, row, 24, design_m3_s)?;
                    flow_error = design_m3_s - q;
                }
                BranchKind::PressureFan { shutoff_pa, k, .. } => {
                    let speed = b.fan_speed_fraction.unwrap_or(1.0);
                    rise = pressure_fan_rise(b, speed, q);
                    s.input(ws, row, 20, k)?;
                    s.input(ws, row, 25, shutoff_pa)?;
                    s.input(ws, row, 26, speed)?;
                    let formula = if let Some((first, last)) = curve_ranges[i] {
                        write_curve(ws, &s, row, b, q, speed, first..=last)?
                    } else {
                        format!("Z{r}*AA{r}^2-U{r}*AU{r}")
                    };
                    s.cell(ws, row, 9, formula, rise)?;
                }
            }
            let pressure_error = if branch_cells[i].is_some() {
                dp + rise - loss
            } else {
                0.0
            };
            s.cell(
                ws,
                row,
                10,
                format!("IF(B{r}=\"Flow fan\",0,H{r}+J{r}-I{r})"),
                pressure_error,
            )?;
            s.cell(
                ws,
                row,
                11,
                format!("IF(B{r}=\"Flow fan\",Y{r}-F{r},0)"),
                flow_error,
            )?;
            let squared = pressure_error.powi(2) + (flow_error / FLOW_SCALE).powi(2);
            s.cell(
                ws,
                row,
                12,
                format!("(K{r}/Inputs!$B$9)^2+(L{r}/Inputs!$B$8)^2"),
                squared,
            )?;
            sum += squared;
            max_pressure_error = max_pressure_error.max(pressure_error.abs());
            max_fixed_error = max_fixed_error.max(flow_error.abs());
        }
    }
    {
        let ws = book.worksheet_from_name("Overview")?;
        s.headers(ws, &["Airframe · Steady airflow", "Value"])?;
        ws.set_column_width(0, 41)?;
        ws.set_column_width(1, 94)?;
        ws.write_string(1, 0, "Project")?;
        ws.write_string(1, 1, name)?;
        ws.write_string(2, 0, "Objective · sum of scaled errors²")?;
        let ranges = |sheet: &str, col: &str, last| {
            if last > 1 {
                format!("{sheet}!{col}2:{col}{last}")
            } else {
                "0".into()
            }
        };
        s.cell(
            ws,
            2,
            1,
            format!(
                "SUM({}, {})",
                ranges("Nodes", "N", n_last),
                ranges("Branches", "M", b_last)
            ),
            sum,
        )?;
        for (row, label, range, value) in [
            (
                3,
                "Maximum internal balance error · m³/s",
                ranges("Nodes", "M", n_last),
                point.max_imbalance,
            ),
            (
                4,
                "Maximum pressure equation error · Pa",
                ranges("Branches", "K", b_last),
                max_pressure_error,
            ),
            (
                5,
                "Maximum fixed flow error · m³/s",
                ranges("Branches", "L", b_last),
                max_fixed_error,
            ),
        ] {
            ws.write_string(row, 0, label)?;
            s.cell(
                ws,
                row,
                1,
                format!("MAX(MAX({range}),-MIN({range}))"),
                value,
            )?;
        }
        let variables = if variable > 2 {
            format!("Inputs!E2:E{}", variable - 1)
        } else {
            "None".into()
        };
        let notes = [
            ("Solver variables", variables),
            ("Edit inputs", "Blue cells are editable. Black formula cells recalculate with their inputs.".into()),
            ("Rebalance", "In Excel Solver, set Overview!B3 to Min with GRG Nonlinear and change the listed trial cells.".into()),
            ("Signed variables", "Uncheck Make Unconstrained Variables Non-Negative. Reverse airflow and negative gauge pressure are valid.".into()),
            ("Larger networks", "Standard Excel Solver allows up to 200 changing cells. For larger networks, re-solve in Airframe and export again.".into()),
            ("Starting values", "Trial values start at the Airframe solution. Editing parameters recalculates equations and residuals; it does not re-solve the network.".into()),
            ("Check convergence", "At balance, internal flow and fixed-flow errors approach 10⁻⁹ m³/s and branch pressure errors approach 10⁻⁶ Pa.".into()),
            ("Node balance", "Net flow = inflow − outflow. Fixed-pressure boundaries can supply or absorb air and have no flow balance constraint.".into()),
            ("Passive branch", "Pressure error = Pfrom − Pto − signed loss. Airflow is positive in the listed direction.".into()),
            ("Opening / door / leakage", "Loss = k·Q·√(Q² + ε²). For a physical opening, k = ρ / [2(Cd·A·open fraction)²].".into()),
            ("Duct / riser", "Loss = blended laminar and turbulent friction + signed dynamic pressure × minor K. Elevations document geometry; no stack pressure is applied.".into()),
            ("Pressure fan", "Pressure error = Pfrom − Pto + fan rise. Rated fans use shutoff×speed² − k·Q·√(Q² + ε²).".into()),
            ("Custom fan", "Interpolate rated pressure at Q/speed, then multiply by speed². Above the last point, extend the last segment.".into()),
            ("Custom fan samples", "Keep rated flow strictly ascending from zero and rated pressure strictly descending to zero. Do not insert or remove rows.".into()),
            ("Fixed-flow fan", "Branch flow equals its commanded flow. The pressure difference is a result, not a constraint.".into()),
            ("Capacitance and inertance", "C = volume / absolute pressure; L = density × length / area. Storage, initial pressures and fan lag do not affect the steady balance.".into()),
        ];
        let wrap = Format::new().set_text_wrap();
        for (i, (label, note)) in notes.into_iter().enumerate() {
            let row = i as u32 + 7;
            ws.write_string(row, 0, label)?;
            ws.write_string_with_format(row, 1, note, &wrap)?;
            ws.set_row_height(row, 32)?;
        }
    }
    book.save(path)
}

fn write_duct(
    ws: &mut Worksheet,
    s: &Styles,
    row: u32,
    d: Duct,
    air: Air,
    q: f64,
) -> Result<f64, XlsxError> {
    let r = row + 1;
    s.input(ws, row, 13, d.length_m)?;
    match d.section {
        Section::Round { diameter_m } => {
            ws.write_string(row, 14, "Round")?;
            s.input(ws, row, 15, diameter_m)?;
        }
        Section::Rectangular { width_m, height_m } => {
            ws.write_string(row, 14, "Rectangular")?;
            s.input(ws, row, 16, width_m)?;
            s.input(ws, row, 17, height_m)?;
        }
    }
    s.input(ws, row, 18, d.roughness_m)?;
    s.input(ws, row, 19, d.minor_k)?;
    let (area, diam) = area_and_diameter(d.section);
    let v = q / area;
    let re = air.density * v.abs() * diam / air.viscosity;
    let dynamic = 0.5 * air.density * v * v.abs();
    let laminar = 32.0 * air.viscosity * d.length_m * v / diam.powi(2);
    let f = if re > 2300.0 {
        0.25 / (d.roughness_m / (3.7 * diam) + 5.74 / re.powf(0.9))
            .log10()
            .powi(2)
    } else {
        0.0
    };
    let loss = duct_loss(d, air, q);
    for (c, formula, value) in [
        (8, format!("AP{r}*(1-AS{r})+AR{r}*AS{r}+AO{r}*T{r}"), loss),
        (
            33,
            format!("IF(O{r}=\"Round\",PI()*P{r}^2/4,Q{r}*R{r})"),
            area,
        ),
        (
            34,
            format!("IF(O{r}=\"Round\",P{r},2*Q{r}*R{r}/(Q{r}+R{r}))"),
            diam,
        ),
        (35, format!("Inputs!$B$3*N{r}/AH{r}"), inertance(d, air)),
        (36, format!("F{r}/AH{r}"), v),
        (39, format!("Inputs!$B$3*ABS(AK{r})*AI{r}/Inputs!$B$4"), re),
        (40, format!("0.5*Inputs!$B$3*AK{r}*ABS(AK{r})"), dynamic),
        (41, format!("32*Inputs!$B$4*N{r}*AK{r}/AI{r}^2"), laminar),
        (
            42,
            format!("IF(AN{r}<=2300,0,0.25/LOG10(S{r}/(3.7*AI{r})+5.74/AN{r}^0.9)^2)"),
            f,
        ),
        (
            43,
            format!("AO{r}*AQ{r}*N{r}/AI{r}"),
            dynamic * f * d.length_m / diam,
        ),
        (
            44,
            format!("MAX(0,MIN(1,(AN{r}-2300)/1700))"),
            ((re - 2300.0) / 1700.0).clamp(0.0, 1.0),
        ),
    ] {
        s.cell(ws, row, c, formula, value)?;
    }
    Ok(loss)
}

fn write_curve(
    ws: &mut Worksheet,
    s: &Styles,
    row: u32,
    b: &Branch,
    q: f64,
    speed: f64,
    rows: std::ops::RangeInclusive<u32>,
) -> Result<String, XlsxError> {
    let r = row + 1;
    let (first, last) = (*rows.start(), *rows.end());
    let q_range = format!("'Fan curves'!$B${first}:$B${last}");
    let p_range = format!("'Fan curves'!$C${first}:$C${last}");
    let shutoff = format!("'Fan curves'!$C${first}");
    let rated = if speed > SPEED_MIN {
        q.max(0.0) / speed
    } else {
        0.0
    };
    let points = &b.custom_fan_curve;
    let segment = points
        .partition_point(|p| p[0] <= rated)
        .clamp(1, points.len() - 1);
    let drag = points[0][1] / points[points.len() - 1][0].powi(2);
    s.cell(
        ws,
        row,
        47,
        format!("IF(AA{r}>Inputs!$B$7,MAX(0,F{r})/AA{r},0)"),
        rated,
    )?;
    s.cell(
        ws,
        row,
        48,
        format!("MIN(ROWS({q_range})-1,MAX(1,MATCH(AV{r},{q_range},1)))"),
        segment as f64,
    )?;
    s.cell(
        ws,
        row,
        49,
        format!("{shutoff}/'Fan curves'!$B${last}^2"),
        drag,
    )?;
    let (qa, qb) = (
        format!("INDEX({q_range},AW{r})"),
        format!("INDEX({q_range},AW{r}+1)"),
    );
    let (pa, pb) = (
        format!("INDEX({p_range},AW{r})"),
        format!("INDEX({p_range},AW{r}+1)"),
    );
    Ok(format!(
        "IF(AA{r}<=Inputs!$B$7,-AX{r}*AU{r},IF(F{r}<0,{shutoff}*AA{r}^2-AX{r}*AU{r},AA{r}^2*({pa}+(AV{r}-{qa})*({pb}-{pa})/({qb}-{qa}))))"
    ))
}
