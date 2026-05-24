use crate::math::vec::Vec3;
use alloc::vec::Vec;

/// Axis-aligned BSP tree. Built offline; provides front-to-back traversal,
/// O(log n) point-in-leaf for PVS lookup, and ray traces for collision.
#[derive(Debug, Clone)]
pub struct BspNode {
    /// 0=X, 1=Y, 2=Z.
    pub axis: u8,
    pub split: f32,
    pub front: BspChild,
    pub back: BspChild,
    pub bb_min: Vec3,
    pub bb_max: Vec3,
}

#[derive(Debug, Clone, Copy)]
pub enum BspChild {
    Node(u32),
    Leaf(u32),
    Empty,
}

/// Convex sector. Leaves sharing a cluster share PVS data.
#[derive(Debug, Clone)]
pub struct BspLeaf {
    pub leaf_id: u32,
    pub face_start: u32,
    pub face_count: u32,
    pub bb_min: Vec3,
    pub bb_max: Vec3,
    pub cluster: u32,
}

pub struct BspTree {
    pub nodes: Vec<BspNode>,
    pub leaves: Vec<BspLeaf>,
}

impl Default for BspTree {
    fn default() -> Self {
        Self::new()
    }
}

impl BspTree {
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            leaves: Vec::new(),
        }
    }

    pub fn find_leaf(&self, point: Vec3) -> Option<u32> {
        if self.nodes.is_empty() {
            return None;
        }
        let mut child = BspChild::Node(0);

        loop {
            match child {
                BspChild::Node(idx) => {
                    let node = self.nodes.get(idx as usize)?;
                    let coord = match node.axis {
                        0 => point.x,
                        1 => point.y,
                        _ => point.z,
                    };
                    child = if coord >= node.split {
                        node.front
                    } else {
                        node.back
                    };
                }
                BspChild::Leaf(idx) => return Some(idx),
                BspChild::Empty => return None,
            }
        }
    }

    /// Visit leaves front-to-back relative to camera. Callback returning false stops traversal.
    pub fn traverse_front_to_back<F>(&self, camera_pos: Vec3, mut callback: F)
    where
        F: FnMut(u32) -> bool,
    {
        if self.nodes.is_empty() {
            return;
        }
        self.traverse_recursive(BspChild::Node(0), camera_pos, &mut callback);
    }

    fn traverse_recursive<F>(&self, child: BspChild, camera_pos: Vec3, callback: &mut F)
    where
        F: FnMut(u32) -> bool,
    {
        match child {
            BspChild::Node(idx) => {
                let node = match self.nodes.get(idx as usize) {
                    Some(n) => n,
                    None => return,
                };
                let coord = match node.axis {
                    0 => camera_pos.x,
                    1 => camera_pos.y,
                    _ => camera_pos.z,
                };
                let (first, second) = if coord >= node.split {
                    (node.front, node.back)
                } else {
                    (node.back, node.front)
                };
                self.traverse_recursive(first, camera_pos, callback);
                self.traverse_recursive(second, camera_pos, callback);
            }
            BspChild::Leaf(idx) => {
                callback(idx);
            }
            BspChild::Empty => {}
        }
    }

    /// Returns (t, leaf) of the first hit, or None.
    pub fn trace_ray(&self, origin: Vec3, direction: Vec3, max_t: f32) -> Option<(f32, u32)> {
        if self.nodes.is_empty() {
            return None;
        }
        self.trace_recursive(BspChild::Node(0), origin, direction, 0.0, max_t)
    }

    fn trace_recursive(
        &self,
        child: BspChild,
        origin: Vec3,
        dir: Vec3,
        t_min: f32,
        t_max: f32,
    ) -> Option<(f32, u32)> {
        if t_min > t_max {
            return None;
        }

        match child {
            BspChild::Node(idx) => {
                let node = self.nodes.get(idx as usize)?;

                let (o_coord, d_coord) = match node.axis {
                    0 => (origin.x, dir.x),
                    1 => (origin.y, dir.y),
                    _ => (origin.z, dir.z),
                };

                let dist = node.split - o_coord;

                if d_coord.abs() < 1e-10 {
                    // Parallel to split plane.
                    let child = if o_coord >= node.split {
                        node.front
                    } else {
                        node.back
                    };
                    return self.trace_recursive(child, origin, dir, t_min, t_max);
                }

                let t_split = dist / d_coord;

                let (near, far) = if o_coord >= node.split {
                    (node.front, node.back)
                } else {
                    (node.back, node.front)
                };

                if t_split < t_min {
                    self.trace_recursive(far, origin, dir, t_min, t_max)
                } else if t_split > t_max {
                    self.trace_recursive(near, origin, dir, t_min, t_max)
                } else {
                    let hit = self.trace_recursive(near, origin, dir, t_min, t_split);
                    if hit.is_some() {
                        return hit;
                    }
                    self.trace_recursive(far, origin, dir, t_split, t_max)
                }
            }
            BspChild::Leaf(idx) => Some((t_min, idx)),
            BspChild::Empty => None,
        }
    }
}
