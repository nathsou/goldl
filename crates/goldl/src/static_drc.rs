//! Quiescent components are already characterized still lifes. Only components
//! whose bounding boxes approach within two cells can jointly change a cell in
//! the next generation. Simulate those connected clusters instead of allocating
//! the entire (potentially tens of millions of cells) static universe.
use crate::phys::Inst;
use goldl_life::{Cell, Pattern};
use std::collections::HashMap;

pub(crate) fn check(insts: &[Inst]) -> Option<Vec<Cell>> {
    const BIN: i64 = 64;
    let mut bins: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    let mut parents: Vec<usize> = (0..insts.len()).collect();
    let mut sizes = vec![1usize; insts.len()];
    let mut seen = vec![usize::MAX; insts.len()];
    let bounds: Vec<_> = insts.iter().map(Inst::bbox).collect();
    fn root(parents: &mut [usize], mut u: usize) -> usize {
        while parents[u] != u {
            parents[u] = parents[parents[u]];
            u = parents[u];
        }
        u
    }
    for (i, &(x0, y0, x1, y1)) in bounds.iter().enumerate() {
        for x in (x0 - 2).div_euclid(BIN)..=(x1 + 2).div_euclid(BIN) {
            for y in (y0 - 2).div_euclid(BIN)..=(y1 + 2).div_euclid(BIN) {
                if let Some(neighbors) = bins.get(&(x, y)) {
                    for &j in neighbors {
                        if seen[j] == i {
                            continue;
                        }
                        seen[j] = i;
                        let (a, b, c, d) = bounds[j];
                        if x0 > c + 2 || a > x1 + 2 || y0 > d + 2 || b > y1 + 2 {
                            continue;
                        }
                        let (mut u, mut v) = (root(&mut parents, i), root(&mut parents, j));
                        if u != v {
                            if sizes[u] < sizes[v] {
                                std::mem::swap(&mut u, &mut v);
                            }
                            parents[v] = u;
                            sizes[u] += sizes[v];
                        }
                    }
                }
            }
        }
        for x in x0.div_euclid(BIN)..=x1.div_euclid(BIN) {
            for y in y0.div_euclid(BIN)..=y1.div_euclid(BIN) {
                bins.entry((x, y)).or_default().push(i);
            }
        }
    }
    let mut clusters: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..insts.len() {
        let r = root(&mut parents, i);
        if sizes[r] > 1 {
            clusters.entry(r).or_default().push(i);
        }
    }
    // Stable ordering keeps diagnostics reproducible.
    let mut clusters: Vec<_> = clusters.into_values().collect();
    clusters.sort_by_key(|c| c[0]);
    for cluster in clusters {
        let mut cells = Vec::new();
        for i in cluster {
            let inst = &insts[i];
            cells.extend(
                inst.oriented()
                    .cells
                    .iter()
                    .map(|&(x, y)| (x + inst.tx, y + inst.ty)),
            );
        }
        let p = Pattern::from_cells(cells);
        let next = p.run(1);
        if p != next {
            let a = p.to_set();
            let b = next.to_set();
            let mut diff: Vec<_> = a.symmetric_difference(&b).copied().collect();
            diff.sort_unstable();
            return Some(diff);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phys::Phys;
    use crate::tech::component::table;

    #[test]
    fn clusters_match_whole_universe_including_overlaps_and_negative_coordinates() {
        let components = table();
        for seed in 0..128usize {
            let insts: Vec<_> = (0..5)
                .map(|j| Inst {
                    comp: ((seed + j * 7) % components.len()) as u16,
                    tx: ((seed * 17 + j * 29) % 101) as i64 - 70,
                    ty: ((seed * 37 + j * 11) % 101) as i64 - 70,
                    dt: 0,
                    trigger: None,
                    group: 0,
                })
                .chain(std::iter::once(Inst {
                    comp: 0,
                    tx: 10000,
                    ty: -10000,
                    dt: 0,
                    trigger: None,
                    group: 0,
                }))
                .collect();
            let ph = Phys {
                insts,
                ..Default::default()
            };
            let p = ph.static_pattern();
            assert_eq!(check(&ph.insts).is_none(), p == p.run(1), "seed {seed}");
        }
        assert!(check(&[]).is_none());
    }
}
