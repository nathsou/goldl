//! Schematic generation: the word-level RTL drawn as a logic diagram with ANSI (distinctive
//! shape) symbols and clean orthogonal wiring.
//!
//! Layered layout: registers act as sources (their D inputs become feedback wires routed
//! below the diagram), layers by longest path, dummy nodes on long edges, barycentric
//! crossing reduction, iterative vertical alignment, and channel routing with left-edge
//! track assignment. The result is a display list consumed by the playground.

use crate::json::Json;
use crate::rtl::{RId, ROp, Rtl};
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct SNode {
    pub kind: &'static str,
    pub label: String,
    /// RTL node (output value), if any.
    pub rid: Option<RId>,
    pub group: u32,
    pub span: (u32, u32),
    pub w: f64,
    pub h: f64,
    pub n_in: usize,
    pub in_names: Vec<String>,
    pub width: u32,
    pub x: f64,
    pub y: f64,
    layer: usize,
    dummy: bool,
}

#[derive(Clone, Debug)]
pub struct SWire {
    pub points: Vec<(f64, f64)>,
    pub width: u32,
    pub rid: RId,
    pub feedback: bool,
    pub src: usize,
    pub dst: usize,
}

pub struct Schematic {
    pub nodes: Vec<SNode>,
    pub wires: Vec<SWire>,
    pub size: (f64, f64),
}

struct Edge {
    src: usize,
    dst: usize,
    port: usize,
    rid: RId,
    width: u32,
    feedback: bool,
}

fn node_shape(op: &ROp) -> (&'static str, f64, f64) {
    match op {
        ROp::Not(_) => ("not", 40.0, 30.0),
        ROp::And(..) => ("and", 44.0, 36.0),
        ROp::Or(..) => ("or", 44.0, 36.0),
        ROp::Xor(..) => ("xor", 48.0, 36.0),
        ROp::RedAnd(_) => ("and", 44.0, 30.0),
        ROp::RedOr(_) => ("or", 44.0, 30.0),
        ROp::RedXor(_) => ("xor", 48.0, 30.0),
        ROp::Mux(..) => ("mux", 34.0, 60.0),
        ROp::Add(..) | ROp::Sub(..) | ROp::Mul(..) => ("arith", 44.0, 44.0),
        ROp::Eq(..) | ROp::Ult(..) | ROp::Slt(..) => ("cmp", 44.0, 40.0),
        ROp::Shl(..) | ROp::Shr(..) | ROp::Sar(..) => ("shift", 44.0, 40.0),
        ROp::Concat(v) => ("concat", 12.0, 14.0 * v.len() as f64 + 12.0),
        ROp::Slice(..) | ROp::Zext(_) | ROp::Sext(_) => ("bus", 12.0, 26.0),
        ROp::Const(_) => ("const", 34.0, 18.0),
        ROp::Input(_) => ("input", 64.0, 22.0),
        ROp::RegQ(_) => ("dff", 56.0, 56.0),
    }
}

fn op_label(op: &ROp, r: &Rtl, n: RId) -> String {
    let w = r.nodes[n as usize].width;
    match op {
        ROp::Add(..) => "+".into(),
        ROp::Sub(..) => "−".into(),
        ROp::Mul(..) => "×".into(),
        ROp::Eq(..) => "=".into(),
        ROp::Ult(..) => "<".into(),
        ROp::Slt(..) => "<ₛ".into(),
        ROp::Shl(..) => "≪".into(),
        ROp::Shr(..) => "≫".into(),
        ROp::Sar(..) => "≫ₛ".into(),
        ROp::Slice(_, lo) => {
            if w == 1 {
                format!("[{lo}]")
            } else {
                format!("[{}:{lo}]", lo + w - 1)
            }
        }
        ROp::Zext(_) => format!("zext {w}"),
        ROp::Sext(_) => format!("sext {w}"),
        ROp::Const(c) => {
            if w <= 4 {
                format!("{c}")
            } else {
                format!("0x{c:x}")
            }
        }
        ROp::Input(p) => r.inputs[*p as usize].name.clone(),
        ROp::RegQ(k) => r.regs[*k as usize].name.clone(),
        ROp::RedAnd(_) => "&".into(),
        ROp::RedOr(_) => "|".into(),
        ROp::RedXor(_) => "^".into(),
        _ => String::new(),
    }
}

/// Build the schematic of `r`.
pub fn build(r: &Rtl) -> Schematic {
    build_with(r, &HashMap::new())
}

/// Build the schematic of `r`, drawing the nodes in `boxes` as labelled boxes for groups.
pub fn build_with(r: &Rtl, boxes: &HashMap<RId, u32>) -> Schematic {
    // Live nodes (from outputs and register next-states).
    let mut live = vec![false; r.nodes.len()];
    let mut stack: Vec<RId> = r
        .outputs
        .iter()
        .map(|o| o.1)
        .chain(r.regs.iter().flat_map(|g| [g.next, g.q]))
        .collect();
    while let Some(n) = stack.pop() {
        if std::mem::replace(&mut live[n as usize], true) {
            continue;
        }
        stack.extend(r.nodes[n as usize].op.args());
    }
    let mut nodes: Vec<SNode> = Vec::new();
    let mut of_rid: HashMap<RId, usize> = HashMap::new();
    let mut edges: Vec<Edge> = Vec::new();
    let mk = |kind: &'static str,
              label: String,
              rid: Option<RId>,
              group: u32,
              span: (u32, u32),
              w: f64,
              h: f64,
              n_in: usize,
              width: u32| SNode {
        kind,
        label,
        rid,
        group,
        span,
        w,
        h,
        n_in,
        in_names: vec![],
        width,
        x: 0.0,
        y: 0.0,
        layer: 0,
        dummy: false,
    };
    for (id, n) in r.nodes.iter().enumerate() {
        if !live[id] || matches!(n.op, ROp::Const(_)) {
            continue;
        }
        let (kind, w, h) = node_shape(&n.op);
        let n_in = match &n.op {
            ROp::RegQ(_) => 1,
            op => op.args().len(),
        };
        if let Some(&g) = boxes.get(&(id as RId)) {
            let gr = &r.groups[g as usize];
            let label = if gr.label.is_empty() {
                gr.name.clone()
            } else {
                gr.label.clone()
            };
            let label: String = if label.chars().count() > 28 {
                label.chars().take(27).chain(['…']).collect()
            } else {
                label
            };
            let w = (24.0 + 7.0 * label.chars().count() as f64).max(70.0);
            let h = (18.0 * (n_in as f64 + 1.0)).max(40.0);
            let mut sn = mk(
                "box",
                label,
                Some(id as RId),
                g,
                (n.span.start, n.span.end),
                w,
                h,
                n_in,
                n.width,
            );
            sn.in_names = vec![String::new(); n_in];
            of_rid.insert(id as RId, nodes.len());
            nodes.push(sn);
            continue;
        }
        let mut sn = mk(
            kind,
            op_label(&n.op, r, id as RId),
            Some(id as RId),
            n.group,
            (n.span.start, n.span.end),
            w,
            h,
            n_in,
            n.width,
        );
        sn.in_names = match &n.op {
            ROp::Mux(..) => vec!["s".into(), "1".into(), "0".into()],
            ROp::RegQ(_) => vec!["D".into()],
            _ => vec![],
        };
        if let ROp::RegQ(k) = n.op {
            let rg = &r.regs[k as usize];
            sn.group = rg.group;
            sn.span = (rg.span.start, rg.span.end);
        }
        of_rid.insert(id as RId, nodes.len());
        nodes.push(sn);
    }
    // Output ports.
    let mut out_nodes = Vec::new();
    for (p, o) in &r.outputs {
        let sn = mk(
            "output",
            p.name.clone(),
            None,
            0,
            (0, 0),
            64.0,
            22.0,
            1,
            p.width,
        );
        out_nodes.push((nodes.len(), *o));
        nodes.push(sn);
    }
    // Edges (constants become one small node per use).
    let add_edge = |nodes: &mut Vec<SNode>,
                    edges: &mut Vec<Edge>,
                    src_rid: RId,
                    dst: usize,
                    port: usize,
                    feedback: bool| {
        let sw = r.nodes[src_rid as usize].width;
        let src = if let ROp::Const(_) = r.nodes[src_rid as usize].op {
            let n = &r.nodes[src_rid as usize];
            let (kind, w, h) = node_shape(&n.op);
            let label = op_label(&n.op, r, src_rid);
            let w = w.max(10.0 + 7.0 * label.chars().count() as f64);
            let grp = nodes[dst].group;
            nodes.push(mk(kind, label, Some(src_rid), grp, (0, 0), w, h, 0, sw));
            nodes.len() - 1
        } else {
            of_rid[&src_rid]
        };
        edges.push(Edge {
            src,
            dst,
            port,
            rid: src_rid,
            width: sw,
            feedback,
        });
    };
    for id in 0..r.nodes.len() {
        let Some(&dst) = of_rid.get(&(id as RId)) else {
            continue;
        };
        match &r.nodes[id].op {
            ROp::RegQ(k) => {
                let next = r.regs[*k as usize].next;
                add_edge(&mut nodes, &mut edges, next, dst, 0, true);
            }
            op => {
                for (k, a) in op.args().into_iter().enumerate() {
                    add_edge(&mut nodes, &mut edges, a, dst, k, false);
                }
            }
        }
    }
    for &(dst, o) in &out_nodes {
        add_edge(&mut nodes, &mut edges, o, dst, 0, false);
    }
    layout(nodes, edges)
}

fn port_in(n: &SNode, k: usize) -> (f64, f64) {
    let y = match n.kind {
        "dff" => n.h * 0.3,
        _ => n.h * (k as f64 + 1.0) / (n.n_in as f64 + 1.0),
    };
    (n.x, (n.y + y).round())
}

fn port_out(n: &SNode) -> (f64, f64) {
    (
        n.x + n.w,
        (n.y + if n.kind == "dff" {
            n.h * 0.3
        } else {
            n.h / 2.0
        })
        .round(),
    )
}

fn layout(mut nodes: Vec<SNode>, edges: Vec<Edge>) -> Schematic {
    let n0 = nodes.len();
    // Layers by longest path over forward edges.
    let mut indeg = vec![0usize; n0];
    let mut succ: Vec<Vec<usize>> = vec![Vec::new(); n0];
    for (k, e) in edges.iter().enumerate() {
        if !e.feedback {
            indeg[e.dst] += 1;
            succ[e.src].push(k);
        }
    }
    let mut queue: Vec<usize> = (0..n0).filter(|&n| indeg[n] == 0).collect();
    let mut qi = 0;
    while qi < queue.len() {
        let n = queue[qi];
        qi += 1;
        for &k in &succ[n] {
            let d = edges[k].dst;
            nodes[d].layer = nodes[d].layer.max(nodes[n].layer + 1);
            indeg[d] -= 1;
            if indeg[d] == 0 {
                queue.push(d);
            }
        }
    }
    // Pull constants next to their consumer; outputs to the last layer.
    let max_layer = nodes.iter().map(|n| n.layer).max().unwrap_or(0);
    for e in &edges {
        if nodes[e.src].kind == "const" {
            nodes[e.src].layer = nodes[e.dst].layer.saturating_sub(1);
        }
    }
    for n in nodes.iter_mut() {
        if n.kind == "output" {
            n.layer = max_layer.max(1);
        }
    }
    let n_layers = nodes.iter().map(|n| n.layer).max().unwrap_or(0) + 1;
    // Chains of dummy nodes for long forward edges: edge k → list of node ids from src to dst.
    // Dummies are shared by all edges of the same source (nets route as trees).
    let mut chains: Vec<Vec<usize>> = Vec::new();
    let mut shared: HashMap<(usize, usize), usize> = HashMap::new();
    for e in &edges {
        let mut ch = vec![e.src];
        if !e.feedback {
            for l in nodes[e.src].layer + 1..nodes[e.dst].layer {
                let id = *shared.entry((e.src, l)).or_insert_with(|| {
                    let mut d = nodes[e.src].clone();
                    d.kind = "dummy";
                    d.dummy = true;
                    d.w = 0.0;
                    d.h = 0.0;
                    d.n_in = 1;
                    d.layer = l;
                    nodes.push(d);
                    nodes.len() - 1
                });
                ch.push(id);
            }
        }
        ch.push(e.dst);
        chains.push(ch);
    }
    // Adjacency between consecutive layers (for ordering).
    let mut up: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
    let mut down: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
    for (k, ch) in chains.iter().enumerate() {
        if edges[k].feedback {
            continue;
        }
        for w in ch.windows(2) {
            if !down[w[0]].contains(&w[1]) {
                down[w[0]].push(w[1]);
                up[w[1]].push(w[0]);
            }
        }
    }
    let mut layers: Vec<Vec<usize>> = vec![Vec::new(); n_layers];
    // Initial order: by group then index (keeps groups together).
    let mut ids: Vec<usize> = (0..nodes.len()).collect();
    ids.sort_by_key(|&n| (nodes[n].group, n));
    for n in ids {
        layers[nodes[n].layer].push(n);
    }
    let mut pos = vec![0f64; nodes.len()];
    let set_pos = |layers: &Vec<Vec<usize>>, pos: &mut Vec<f64>| {
        for l in layers {
            for (k, &n) in l.iter().enumerate() {
                pos[n] = k as f64;
            }
        }
    };
    set_pos(&layers, &mut pos);
    for it in 0..24 {
        let downward = it % 2 == 0;
        let range: Vec<usize> = if downward {
            (1..n_layers).collect()
        } else {
            (0..n_layers.saturating_sub(1)).rev().collect()
        };
        for l in range {
            let nb = if downward { &up } else { &down };
            let mut keyed: Vec<(f64, usize)> = layers[l]
                .iter()
                .map(|&n| {
                    let v = &nb[n];
                    let b = if v.is_empty() {
                        pos[n]
                    } else {
                        v.iter().map(|&m| pos[m]).sum::<f64>() / v.len() as f64
                    };
                    (b, n)
                })
                .collect();
            keyed.sort_by(|a, b| {
                a.0.partial_cmp(&b.0)
                    .unwrap()
                    .then(nodes[a.1].group.cmp(&nodes[b.1].group))
            });
            layers[l] = keyed.into_iter().map(|x| x.1).collect();
            for (k, &n) in layers[l].iter().enumerate() {
                pos[n] = k as f64;
            }
        }
    }
    // Vertical positions: stack, then align with neighbours while keeping order.
    const GAP: f64 = 18.0;
    for l in &layers {
        let mut y = 0.0;
        for &n in l {
            nodes[n].y = y;
            y += nodes[n].h + GAP;
        }
    }
    // Median alignment sweeps: each node wants its wires straight; order within a layer is
    // kept by pushing nodes down past their predecessor in the layer.
    // Hop list: (from node, to node, input port at `to`).
    let mut hops: Vec<(usize, usize, usize)> = Vec::new();
    for (k, ch) in chains.iter().enumerate() {
        if edges[k].feedback {
            continue;
        }
        for w in ch.windows(2) {
            let port = if w[1] == edges[k].dst {
                edges[k].port
            } else {
                0
            };
            if !hops.contains(&(w[0], w[1], port)) {
                hops.push((w[0], w[1], port));
            }
        }
    }
    let mut preds: Vec<Vec<(usize, usize)>> = vec![Vec::new(); nodes.len()];
    let mut succs: Vec<Vec<(usize, usize)>> = vec![Vec::new(); nodes.len()];
    for &(a, b, p) in &hops {
        preds[b].push((a, p));
        succs[a].push((b, p));
    }
    let out_off = |n: &SNode| if n.dummy { 0.0 } else { port_out(n).1 - n.y };
    let in_off = |n: &SNode, p: usize| if n.dummy { 0.0 } else { port_in(n, p).1 - n.y };
    let median = |mut v: Vec<f64>| -> Option<f64> {
        if v.is_empty() {
            return None;
        }
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let m = v.len() / 2;
        Some(if v.len() % 2 == 1 {
            v[m]
        } else {
            (v[m - 1] + v[m]) / 2.0
        })
    };
    let gap = |a: &SNode, b: &SNode| {
        if a.dummy && b.dummy {
            8.0
        } else if a.dummy || b.dummy {
            12.0
        } else {
            GAP
        }
    };
    // Height budget: the tallest stacked layer, with some air.
    let h_max = layers
        .iter()
        .map(|l| {
            let mut h = 0.0;
            for (k, &n) in l.iter().enumerate() {
                h += nodes[n].h
                    + if k > 0 {
                        gap(&nodes[l[k - 1]], &nodes[n])
                    } else {
                        0.0
                    };
            }
            h
        })
        .fold(0.0, f64::max)
        * 1.25
        + 40.0;
    for l in &layers {
        let mut y = 0.0;
        for (k, &n) in l.iter().enumerate() {
            if k > 0 {
                y += gap(&nodes[l[k - 1]], &nodes[n]);
            }
            nodes[n].y = y;
            y += nodes[n].h;
        }
    }
    for pass in 0..5 {
        let up_pass = pass % 2 == 1;
        let order: Vec<usize> = if up_pass {
            (0..n_layers).rev().collect()
        } else {
            (0..n_layers).collect()
        };
        for l in order {
            let mut prev: Option<usize> = None;
            for idx in 0..layers[l].len() {
                let n = layers[l][idx];
                let want = if up_pass || preds[n].is_empty() {
                    median(
                        succs[n]
                            .iter()
                            .map(|&(b, p)| nodes[b].y + in_off(&nodes[b], p) - out_off(&nodes[n]))
                            .collect(),
                    )
                } else {
                    median(
                        preds[n]
                            .iter()
                            .map(|&(a, p)| nodes[a].y + out_off(&nodes[a]) - in_off(&nodes[n], p))
                            .collect(),
                    )
                };
                let lo = prev.map_or(0.0, |p| nodes[p].y + nodes[p].h + gap(&nodes[p], &nodes[n]));
                let y = want.unwrap_or(nodes[n].y).max(lo);
                nodes[n].y = y;
                prev = Some(n);
            }
            // Keep the layer within the height of the tallest stack.
            let mut next_top = h_max;
            for idx in (0..layers[l].len()).rev() {
                let n = layers[l][idx];
                if nodes[n].y + nodes[n].h > next_top {
                    nodes[n].y = next_top - nodes[n].h;
                }
                let g = if idx > 0 {
                    gap(&nodes[layers[l][idx - 1]], &nodes[n])
                } else {
                    0.0
                };
                next_top = nodes[n].y - g;
            }
        }
    }
    let y_min = nodes.iter().map(|n| n.y).fold(f64::INFINITY, f64::min);
    for n in nodes.iter_mut() {
        n.y -= y_min - 20.0;
        n.y = (n.y / 2.0).round() * 2.0;
    }
    let y_max = nodes.iter().map(|n| n.y + n.h).fold(0.0, f64::max);
    if std::env::var("GOLDL_DEBUG").is_ok() {
        for (l, ln) in layers.iter().enumerate() {
            let ys: Vec<String> = ln
                .iter()
                .map(|&n| {
                    format!(
                        "{}{}@{:.0}",
                        nodes[n].kind.chars().next().unwrap(),
                        if nodes[n].dummy { "" } else { "" },
                        nodes[n].y
                    )
                })
                .collect();
            eprintln!("layer {l}: {}", ys.join(" "));
        }
    }
    // Channel track assignment: per channel (after layer l), vertical segments as intervals.
    // Segment = (edge index, hop index, y0, y1).
    let mut chan: Vec<Vec<(usize, usize, f64, f64)>> = vec![Vec::new(); n_layers + 1];
    let fb_lane = |k: usize| y_max + 30.0 + 10.0 * k as f64;
    let mut fb_index = HashMap::new();
    for (k, e) in edges.iter().enumerate() {
        if e.feedback {
            fb_index.insert(k, fb_index.len());
        }
    }
    let y_out = |n: &SNode| if n.dummy { n.y } else { port_out(n).1 };
    let y_in = |n: &SNode, port: usize| if n.dummy { n.y } else { port_in(n, port).1 };
    for (k, ch) in chains.iter().enumerate() {
        let e = &edges[k];
        if e.feedback {
            let lane = fb_lane(fb_index[&k]);
            let s = &nodes[e.src];
            let d = &nodes[e.dst];
            chan[s.layer + 1].push((k, 0, y_out(s), lane));
            chan[d.layer].push((k, 1, lane, y_in(d, e.port)));
            continue;
        }
        for (h, w) in ch.windows(2).enumerate() {
            let (a, b) = (&nodes[w[0]], &nodes[w[1]]);
            let port = if w[1] == e.dst { e.port } else { 0 };
            chan[b.layer].push((k, h, y_out(a), y_in(b, port)));
        }
    }
    // Left-edge algorithm; segments of the same source net may share a track.
    let mut track: HashMap<(usize, usize), usize> = HashMap::new();
    let mut n_tracks = vec![0usize; n_layers + 1];
    for (c, segs) in chan.iter().enumerate() {
        let mut v: Vec<&(usize, usize, f64, f64)> =
            segs.iter().filter(|s| (s.2 - s.3).abs() > 0.5).collect();
        v.sort_by(|a, b| a.2.min(a.3).partial_cmp(&b.2.min(b.3)).unwrap());
        let mut tracks: Vec<(f64, usize)> = Vec::new(); // (end y, source node)
        for s in v {
            let (lo, hi) = (s.2.min(s.3), s.2.max(s.3));
            let src = edges[s.0].src;
            let t = tracks
                .iter()
                .position(|&(end, sn)| end + 6.0 < lo || (sn == src && !edges[s.0].feedback))
                .unwrap_or_else(|| {
                    tracks.push((f64::NEG_INFINITY, usize::MAX));
                    tracks.len() - 1
                });
            tracks[t] = (tracks[t].0.max(hi), src);
            track.insert((s.0, s.1), t);
        }
        n_tracks[c] = tracks.len();
    }
    // Layer x positions.
    let mut layer_x = vec![0f64; n_layers];
    let mut chan_x = vec![0f64; n_layers + 1];
    let mut x = 20.0 + 14.0 * n_tracks[0] as f64;
    chan_x[0] = 10.0;
    for l in 0..n_layers {
        let wmax = layers[l].iter().map(|&n| nodes[n].w).fold(0.0, f64::max);
        layer_x[l] = x;
        chan_x[l + 1] = x + wmax + 16.0;
        x += wmax + 32.0 + 12.0 * n_tracks[l + 1] as f64;
    }
    for n in nodes.iter_mut() {
        let wmax = layers[n.layer].iter().count();
        let _ = wmax;
        n.x = layer_x[n.layer];
    }
    // Right-align narrow nodes in their layer would bend wires; keep left aligned but put
    // dummies' x at the layer start and extend through the layer width.
    let track_x = |c: usize, t: usize| chan_x[c] + 12.0 * t as f64;
    let mut wires = Vec::new();
    for (k, ch) in chains.iter().enumerate() {
        let e = &edges[k];
        let mut pts: Vec<(f64, f64)> = Vec::new();
        let s = &nodes[e.src];
        let (sx, sy) = port_out(s);
        pts.push((sx, sy));
        if e.feedback {
            let lane = fb_lane(fb_index[&k]);
            let d = &nodes[e.dst];
            let (dx, dy) = port_in(d, e.port);
            let x1 = track_x(s.layer + 1, track.get(&(k, 0)).copied().unwrap_or(0));
            let x2 = track_x(d.layer, track.get(&(k, 1)).copied().unwrap_or(0));
            pts.extend([(x1, sy), (x1, lane), (x2, lane), (x2, dy), (dx, dy)]);
        } else {
            let mut cy = sy;
            for (h, w) in ch.windows(2).enumerate() {
                let b = &nodes[w[1]];
                let port = if w[1] == e.dst { e.port } else { 0 };
                let (bx, by) = if b.dummy {
                    (b.x, b.y)
                } else {
                    port_in(b, port)
                };
                if (by - cy).abs() > 0.5 {
                    let tx = track_x(b.layer, track.get(&(k, h)).copied().unwrap_or(0));
                    pts.push((tx, cy));
                    pts.push((tx, by));
                }
                pts.push((bx, by));
                if b.dummy {
                    // Pass through the dummy's layer.
                    let wmax = layers[b.layer]
                        .iter()
                        .map(|&n| nodes[n].w)
                        .fold(0.0, f64::max);
                    pts.push((b.x + wmax, by));
                }
                cy = by;
            }
        }
        // Remove collinear points.
        let mut clean: Vec<(f64, f64)> = Vec::new();
        for p in pts {
            if clean.last() == Some(&p) {
                continue;
            }
            if clean.len() >= 2 {
                let a = clean[clean.len() - 2];
                let b = clean[clean.len() - 1];
                if (a.0 == b.0 && b.0 == p.0) || (a.1 == b.1 && b.1 == p.1) {
                    clean.pop();
                }
            }
            clean.push(p);
        }
        wires.push(SWire {
            points: clean,
            width: e.width,
            rid: e.rid,
            feedback: e.feedback,
            src: e.src,
            dst: e.dst,
        });
    }
    let n_fb = fb_index.len();
    let w = chan_x[n_layers] + 40.0;
    let h = y_max + 40.0 + 10.0 * n_fb as f64;
    nodes.truncate(n0.max(nodes.iter().rposition(|n| !n.dummy).map_or(0, |p| p + 1)));
    Schematic {
        nodes,
        wires,
        size: (w, h),
    }
}

impl Schematic {
    pub fn to_json(&self, r: &Rtl) -> Json {
        self.to_json_mapped(r, &|x| Some(x))
    }

    /// JSON with node and wire `rid`s translated by `map` (e.g. to the design's node ids).
    pub fn to_json_mapped(&self, r: &Rtl, map: &dyn Fn(RId) -> Option<RId>) -> Json {
        let nodes: Vec<Json> = self
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| !n.dummy)
            .map(|(k, n)| {
                let ins: Vec<Json> = (0..n.n_in)
                    .map(|p| {
                        let (x, y) = port_in(n, p);
                        Json::Arr(vec![x.into(), y.into()])
                    })
                    .collect();
                let (ox, oy) = port_out(n);
                Json::obj()
                    .with("id", k)
                    .with("kind", n.kind)
                    .with("label", n.label.clone())
                    .with(
                        "rid",
                        n.rid
                            .and_then(|x| if n.kind == "box" { None } else { map(x) })
                            .map(|x| x as i64),
                    )
                    .with("group", n.group)
                    .with("span", vec![n.span.0, n.span.1])
                    .with("x", n.x)
                    .with("y", n.y)
                    .with("w", n.w)
                    .with("h", n.h)
                    .with("width", n.width)
                    .with("ins", ins)
                    .with("inNames", n.in_names.clone())
                    .with("out", vec![ox, oy])
            })
            .collect();
        let wires: Vec<Json> = self
            .wires
            .iter()
            .map(|w| {
                let pts: Vec<Json> = w
                    .points
                    .iter()
                    .map(|&(x, y)| Json::Arr(vec![x.into(), y.into()]))
                    .collect();
                Json::obj()
                    .with("points", pts)
                    .with("width", w.width)
                    .with("rid", map(w.rid).map(|x| x as i64))
                    .with("feedback", w.feedback)
                    .with("src", w.src)
                    .with("dst", w.dst)
            })
            .collect();
        let groups: Vec<Json> = r
            .groups
            .iter()
            .enumerate()
            .map(|(k, g)| {
                Json::obj()
                    .with("id", k)
                    .with("name", g.name.clone())
                    .with("label", g.label.clone())
                    .with("kind", g.kind.clone())
                    .with("parent", g.parent.map(|p| p as i64))
            })
            .collect();
        Json::obj()
            .with("nodes", nodes)
            .with("wires", wires)
            .with("groups", groups)
            .with("size", vec![self.size.0, self.size.1])
    }
}

/// Gate-level view of one RTL operator: a small netlist of 1-bit AND/OR/XOR/NOT gates
/// (inputs are the operator's argument bits) and how to feed it from the design's values.
pub struct GateView {
    pub rtl: Rtl,
    /// Input port k of `rtl` is bit `.1` of design node `.0`.
    pub inputs: Vec<(RId, u32)>,
}

/// Build the gate-level view of RTL node `rid` (None for sources and pure wiring with no gates).
pub fn gate_level(r: &Rtl, rid: RId) -> Option<GateView> {
    use crate::aig::{is_neg, neg, Aig, GateKind, Lit, FALSE, TRUE};
    let n = &r.nodes[rid as usize];
    if matches!(n.op, ROp::Input(_) | ROp::RegQ(_) | ROp::Const(_)) {
        return None;
    }
    let mut a = Aig::new();
    a.log = Some(Vec::new());
    let mut mini = Rtl::new("gates");
    let mut inputs: Vec<(RId, u32)> = Vec::new();
    let mut bits: Vec<Vec<Lit>> = vec![Vec::new(); r.nodes.len()];
    let mut lit_node: HashMap<Lit, RId> = HashMap::new();
    let args = n.op.args();
    let letters = |k: usize| -> String {
        match (&n.op, k) {
            (ROp::Mux(..), 0) => "s".into(),
            (ROp::Mux(..), 1) => "a".into(),
            (ROp::Mux(..), _) => "b".into(),
            (_, k) => ((b'a' + (k as u8).min(25)) as char).to_string(),
        }
    };
    for (k, &x) in args.iter().enumerate() {
        if !bits[x as usize].is_empty() {
            continue;
        }
        let w = r.nodes[x as usize].width;
        if let Some(c) = r.const_val(x) {
            bits[x as usize] = (0..w)
                .map(|b| if (c >> b) & 1 == 1 { TRUE } else { FALSE })
                .collect();
            continue;
        }
        let name = r
            .probes
            .iter()
            .find(|p| p.1 == x)
            .map(|p| p.0.clone())
            .unwrap_or_else(|| letters(k));
        let mut v = Vec::new();
        for b in 0..w {
            let port = mini.inputs.len() as u32;
            mini.inputs.push(crate::gnl::PortInfo {
                name: if w == 1 {
                    name.clone()
                } else {
                    format!("{name}[{b}]")
                },
                width: 1,
            });
            let l = a.input(port, 0);
            let id = mini.input(port, 1, crate::syntax::Span::default());
            lit_node.insert(l, id);
            inputs.push((x, b));
            v.push(l);
        }
        bits[x as usize] = v;
    }
    let out = crate::aig::blast_op(&mut a, r, n, &bits);
    let log = a.log.take().unwrap_or_default();
    if log.is_empty() {
        return None;
    }
    fn get(mini: &mut Rtl, lit_node: &mut HashMap<Lit, RId>, l: Lit) -> RId {
        if l == TRUE || l == FALSE {
            return mini.konst((l == TRUE) as u64, 1);
        }
        if let Some(&id) = lit_node.get(&l) {
            return id;
        }
        if lit_node.contains_key(&neg(l)) || is_neg(l) {
            let x = get(mini, lit_node, neg(l));
            let id = mini.gate(ROp::Not(x), 1, 0, crate::syntax::Span::default());
            lit_node.insert(l, id);
            return id;
        }
        // An internal literal never logged (should not happen): show it as a constant.
        mini.konst(0, 1)
    }
    for gl in &log {
        if gl.out == gl.a
            || gl.out == gl.b
            || gl.out == TRUE
            || gl.out == FALSE
            || lit_node.contains_key(&gl.out)
        {
            continue;
        }
        let x = get(&mut mini, &mut lit_node, gl.a);
        let y = get(&mut mini, &mut lit_node, gl.b);
        let op = match gl.kind {
            GateKind::And => ROp::And(x, y),
            GateKind::Or => ROp::Or(x, y),
            GateKind::Xor => ROp::Xor(x, y),
        };
        let id = mini.gate(op, 1, 0, crate::syntax::Span::default());
        lit_node.insert(gl.out, id);
    }
    for (k, &l) in out.iter().enumerate() {
        let id = get(&mut mini, &mut lit_node, l);
        let name = if out.len() == 1 {
            "y".to_string()
        } else {
            format!("y[{k}]")
        };
        mini.outputs
            .push((crate::gnl::PortInfo { name, width: 1 }, id));
    }
    Some(GateView { rtl: mini, inputs })
}

/// Scoped view of one group of the design.
pub struct ScopeView {
    pub rtl: Rtl,
    /// Design node shown by each node of `rtl` (None for constants and boxes).
    pub orig: Vec<Option<RId>>,
    /// Collapsed sub-groups: box node -> group.
    pub boxes: HashMap<RId, u32>,
}

/// Scoped view of the design: the RTL nodes of group `scope`, with each direct sub-group
/// collapsed into one box, signals entering from outside as input ports and signals used
/// outside (or design outputs) as output ports.
pub fn scoped(r: &Rtl, scope: u32) -> ScopeView {
    let child_of = |mut g: u32| -> Option<u32> {
        let mut prev = scope;
        for _ in 0..256 {
            if g == scope {
                return Some(prev);
            }
            prev = g;
            g = r.groups.get(g as usize)?.parent?;
        }
        None
    };
    let mut live = vec![false; r.nodes.len()];
    let mut stack: Vec<RId> = r
        .outputs
        .iter()
        .map(|o| o.1)
        .chain(r.regs.iter().flat_map(|g| [g.next, g.q]))
        .collect();
    while let Some(n) = stack.pop() {
        if std::mem::replace(&mut live[n as usize], true) {
            continue;
        }
        stack.extend(r.nodes[n as usize].op.args());
    }
    let group_of = |i: usize| match r.nodes[i].op {
        ROp::RegQ(k) => r.regs[k as usize].group,
        _ => r.nodes[i].group,
    };
    let owner: Vec<Option<u32>> = (0..r.nodes.len())
        .map(|i| {
            if live[i] && !matches!(r.nodes[i].op, ROp::Const(_) | ROp::Input(_)) {
                child_of(group_of(i))
            } else {
                None
            }
        })
        .collect();
    let order = r.live_order();
    let mut members: Vec<(u32, Vec<RId>)> = Vec::new();
    for &id in &order {
        if let Some(o) = owner[id as usize] {
            if o != scope {
                match members.iter_mut().find(|m| m.0 == o) {
                    Some(m) => m.1.push(id),
                    None => members.push((o, vec![id])),
                }
            }
        }
    }
    let mut mini = Rtl::new(&r.groups[scope as usize].name);
    let mut orig: Vec<Option<RId>> = Vec::new();
    let mut node_map: HashMap<RId, RId> = HashMap::new();
    let name_of = |x: RId| -> String {
        r.probes
            .iter()
            .find(|p| p.1 == x)
            .map(|p| p.0.clone())
            .unwrap_or_else(|| match &r.nodes[x as usize].op {
                ROp::Input(p) => r.inputs[*p as usize].name.clone(),
                ROp::RegQ(k) => r.regs[*k as usize].name.clone(),
                op => op.name().to_string(),
            })
    };
    // Design signal -> node of the scoped netlist (an input port when it comes from outside).
    let sig = |mini: &mut Rtl,
               orig: &mut Vec<Option<RId>>,
               node_map: &mut HashMap<RId, RId>,
               x: RId|
     -> RId {
        if let Some(&m) = node_map.get(&x) {
            return m;
        }
        let w = r.nodes[x as usize].width;
        let m = if let Some(c) = r.const_val(x) {
            mini.konst(c, w)
        } else {
            let port = mini.inputs.len() as u32;
            mini.inputs.push(crate::gnl::PortInfo {
                name: name_of(x),
                width: w,
            });
            let id = mini.input(port, w, crate::syntax::Span::default());
            node_map.insert(x, id);
            id
        };
        while orig.len() < mini.nodes.len() {
            orig.push(None);
        }
        orig[m as usize] = Some(x);
        m
    };
    let mut boxes: HashMap<RId, u32> = HashMap::new();
    let mut box_node: HashMap<u32, RId> = HashMap::new();
    let mut regs: Vec<(usize, RId)> = Vec::new();
    for &id in &order {
        let Some(o) = owner[id as usize] else {
            continue;
        };
        let n = &r.nodes[id as usize];
        let m = if o == scope {
            match &n.op {
                ROp::RegQ(k) => {
                    let mut g = r.regs[*k as usize].clone();
                    let kk = mini.regs.len() as u32;
                    let q = mini.reg_q(kk, n.width, 0, g.span);
                    g.q = q;
                    g.group = 0;
                    mini.regs.push(g);
                    regs.push((*k as usize, q));
                    q
                }
                op => {
                    let mut op = op.clone();
                    remap_args(&mut op, &mut |x| {
                        sig(&mut mini, &mut orig, &mut node_map, x)
                    });
                    mini.gate(op, n.width, 0, n.span)
                }
            }
        } else if let Some(&b) = box_node.get(&o) {
            b
        } else {
            let mbrs = &members.iter().find(|m| m.0 == o).unwrap().1;
            let inside: std::collections::HashSet<RId> = mbrs.iter().copied().collect();
            let mut ins: Vec<RId> = Vec::new();
            for &mbr in mbrs {
                let args = match r.nodes[mbr as usize].op {
                    ROp::RegQ(k) => vec![r.regs[k as usize].next],
                    ref op => op.args(),
                };
                for x in args {
                    if !inside.contains(&x) && r.const_val(x).is_none() && !ins.contains(&x) {
                        ins.push(x);
                    }
                }
            }
            let args: Vec<RId> = ins
                .iter()
                .map(|&x| sig(&mut mini, &mut orig, &mut node_map, x))
                .collect();
            let gr = &r.groups[o as usize];
            let sp = gr
                .span
                .map(|(a, b)| crate::syntax::Span { start: a, end: b })
                .unwrap_or_default();
            let w = mbrs
                .iter()
                .map(|&x| r.nodes[x as usize].width)
                .max()
                .unwrap_or(1);
            let b = mini.gate(ROp::Concat(args), w, o, sp);
            boxes.insert(b, o);
            box_node.insert(o, b);
            b
        };
        node_map.insert(id, m);
        while orig.len() < mini.nodes.len() {
            orig.push(None);
        }
        if !boxes.contains_key(&m) {
            orig[m as usize] = Some(id);
        }
    }
    for (k, q) in regs {
        let nx = r.regs[k].next;
        let m = sig(&mut mini, &mut orig, &mut node_map, nx);
        if let Some(g) = mini.regs.iter_mut().find(|g| g.q == q) {
            g.next = m;
        }
    }
    // Outputs: signals of the scope used outside it (or design outputs).
    let mut used_out = vec![false; r.nodes.len()];
    for (i, n) in r.nodes.iter().enumerate() {
        if live[i] && owner[i].is_none() {
            for a in n.op.args() {
                used_out[a as usize] = true;
            }
        }
    }
    for (_, o) in &r.outputs {
        used_out[*o as usize] = true;
    }
    for g in &r.regs {
        if owner[g.q as usize].is_none() {
            used_out[g.next as usize] = true;
        }
    }
    let mut seen: Vec<RId> = Vec::new();
    for &id in &order {
        if owner[id as usize].is_none() || !used_out[id as usize] {
            continue;
        }
        let m = node_map[&id];
        if seen.contains(&m) {
            continue;
        }
        seen.push(m);
        let name = r
            .outputs
            .iter()
            .find(|o| o.1 == id)
            .map(|o| o.0.name.clone())
            .unwrap_or_else(|| name_of(id));
        mini.outputs.push((
            crate::gnl::PortInfo {
                name,
                width: r.nodes[id as usize].width,
            },
            m,
        ));
    }
    while orig.len() < mini.nodes.len() {
        orig.push(None);
    }
    mini.groups = r.groups.clone();
    mini.probes = r
        .probes
        .iter()
        .filter_map(|p| node_map.get(&p.1).map(|&m| (p.0.clone(), m, 0)))
        .collect();
    ScopeView {
        rtl: mini,
        orig,
        boxes,
    }
}

fn remap_args(op: &mut ROp, f: &mut dyn FnMut(RId) -> RId) {
    use ROp::*;
    match op {
        Const(_) | Input(_) | RegQ(_) => {}
        Not(a) | Slice(a, _) | Zext(a) | Sext(a) | RedAnd(a) | RedOr(a) | RedXor(a) => *a = f(*a),
        And(a, b)
        | Or(a, b)
        | Xor(a, b)
        | Add(a, b)
        | Sub(a, b)
        | Mul(a, b)
        | Shl(a, b)
        | Shr(a, b)
        | Sar(a, b)
        | Eq(a, b)
        | Ult(a, b)
        | Slt(a, b) => {
            *a = f(*a);
            *b = f(*b);
        }
        Mux(s, a, b) => {
            *s = f(*s);
            *a = f(*a);
            *b = f(*b);
        }
        Concat(v) => v.iter_mut().for_each(|x| *x = f(*x)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alu_schematic() {
        let src = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/alu.goldl"
        ))
        .unwrap();
        let (rtl, _) =
            crate::elab::elaborate(&src, None).unwrap_or_else(|a| panic!("{:?}", a.diags));
        let s = build(&rtl);
        assert!(s.nodes.iter().filter(|n| !n.dummy).count() > 20);
        // Every wire is orthogonal.
        for w in &s.wires {
            for p in w.points.windows(2) {
                assert!(
                    p[0].0 == p[1].0 || p[0].1 == p[1].1,
                    "diagonal segment {:?}",
                    p
                );
            }
        }
        let j = s.to_json(&rtl).to_string();
        assert!(j.len() > 1000);
    }

    #[test]
    fn gate_level_matches_rtl() {
        let src = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/alu.goldl"
        ))
        .unwrap();
        let (rtl, _) =
            crate::elab::elaborate(&src, None).unwrap_or_else(|a| panic!("{:?}", a.diags));
        let mut seed = 0x1234_5678u64;
        let mut rnd = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let mut n_views = 0;
        for id in rtl.live_order() {
            let Some(v) = gate_level(&rtl, id) else {
                continue;
            };
            n_views += 1;
            let _ = build(&v.rtl);
            for _ in 0..16 {
                // Random argument values.
                let n = &rtl.nodes[id as usize];
                let args = n.op.args();
                let vals: Vec<u64> = args
                    .iter()
                    .map(|&a| {
                        rtl.const_val(a).unwrap_or_else(|| {
                            rnd() & crate::rtl::mask(rtl.nodes[a as usize].width)
                        })
                    })
                    .collect();
                let ws: Vec<u32> = args.iter().map(|&a| rtl.nodes[a as usize].width).collect();
                let want = crate::rtl::eval_op(&n.op, &vals, &ws, n.width);
                let ins: Vec<u64> = v
                    .inputs
                    .iter()
                    .map(|&(x, b)| (vals[args.iter().position(|&a| a == x).unwrap()] >> b) & 1)
                    .collect();
                let mut sim = crate::rtl::RtlSim::new(&v.rtl);
                let outs = sim.step(&v.rtl, &ins);
                let got = outs
                    .iter()
                    .enumerate()
                    .fold(0u64, |acc, (k, &b)| acc | (b << k));
                assert_eq!(got, want, "node {id} {:?}", n.op);
            }
        }
        assert!(n_views > 10);
    }

    #[test]
    fn scoped_views() {
        let src = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/cpu.goldl"
        ))
        .unwrap();
        let (rtl, _) =
            crate::elab::elaborate(&src, None).unwrap_or_else(|a| panic!("{:?}", a.diags));
        for g in 0..rtl.groups.len() as u32 {
            let v = scoped(&rtl, g);
            let s = build_with(&v.rtl, &v.boxes);
            for w in &s.wires {
                for p in w.points.windows(2) {
                    assert!(
                        p[0].0 == p[1].0 || p[0].1 == p[1].1,
                        "diagonal segment {:?}",
                        p
                    );
                }
            }
            if g == 0 {
                assert!(!v.boxes.is_empty());
                assert_eq!(v.rtl.outputs.len(), rtl.outputs.len());
            }
        }
    }
}
