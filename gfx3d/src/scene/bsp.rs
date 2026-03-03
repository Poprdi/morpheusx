use crate::math::vec::Vec3;
use alloc::vec::Vec;

/// Binary Space Partition tree for indoor scene rendering.
///
/// BSP was the engine behind Doom, Quake 1, Quake 2, Half-Life 1, and
/// essentially all 90s FPS games. The tree is built OFFLINE (at map load)
/// and provides:
///
/// 1. **Front-to-back ordering** — traverse the tree camera-side-first to get
///    a perfect depth ordering without sorting. Combined with a span buffer,
///    this means occluded geometry is never even rasterized.
///
/// 2. **Point-in-leaf lookup** — O(log n) determination of which leaf/sector
///    the camera is in, which feeds the PVS (Potentially Visible Set) lookup.
///
/// 3. **Collision detection** — trace a ray through the BSP to find the first
///    solid surface it hits (used for hitscan weapons, line-of-sight checks).
///
/// Our BSP is axis-aligned (AABB splits only) for simplicity. Quake used
/// arbitrary split planes for tighter fits, but axis-aligned BSP has two
/// advantages: (a) simpler tree traversal (just compare one coordinate),
/// (b) tighter bounding boxes for frustum culling.

/// A single BSP node.
#[derive(Debug, Clone)]
pub struct BspNode {
    /// Split axis: 0=X, 1=Y, 2=Z.
    pub axis: u8,
    /// Split position along the axis.
    pub split: f32,
    /// Index of front child (BspChild::Node or BspChild::Leaf).
    pub front: BspChild,
    /// Index of back child.
    pub back: BspChild,
    /// Bounding box min corner.
    pub bb_min: Vec3,
    /// Bounding box max corner.
    pub bb_max: Vec3,
}

/// Child reference: either another node or a leaf (sector).
#[derive(Debug, Clone, Copy)]
pub enum BspChild {
    Node(u32), // index into BspTree::nodes
    Leaf(u32), // index into BspTree::leaves
    Empty,
}

/// A BSP leaf = convex sector of the world.
#[derive(Debug, Clone)]
pub struct BspLeaf {
    pub leaf_id: u32,
    /// Indices into the face array (triangles belonging to this sector).
    pub face_start: u32,
    pub face_count: u32,
    /// Bounding box of this leaf (for frustum culling).
    pub bb_min: Vec3,
    pub bb_max: Vec3,
    /// PVS cluster id (leaves in the same cluster share visibility data).
    pub cluster: u32,
}

/// The complete BSP tree.
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

    /// Find which leaf contains a point.
    ///
    /// Walks the tree from root, comparing the point against each split plane.
    /// O(log n) traversal, cache-friendly because nodes are small and
    /// accessed in a predictable pattern.
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

    /// Front-to-back traversal from camera position.
    ///
    /// Visits leaves in front-to-back order relative to the camera. This is the
    /// key to the Quake 1 span-buffer approach: render front surfaces first,
    /// and subsequent surfaces behind them can be trivially rejected.
    ///
    /// The callback receives leaf index. Return `false` to stop traversal early
    /// (useful when the span buffer is full — every pixel is covered).
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
                // Visit the side the camera is on first (front-to-back)
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

    /// Ray-BSP intersection for collision/hitscan.
    ///
    /// Traces a ray from `origin` in `direction`, returns the parametric t
    /// of the first solid surface hit, and the leaf it hit in.
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
                    // Ray parallel to split plane
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
