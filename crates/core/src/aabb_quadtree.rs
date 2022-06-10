#![deny(missing_docs)]

//! A simple spacial partitioning data structure that allows fast queries for
//! 2-dimensional objects.
//!
//! As the name implies, the tree is a mapping from axis-aligned-bounding-box => object.

use euclid::{Point2D as TypedPoint2D, Rect as TypedRect, Size2D as TypedSize2D};
use fnv::FnvHasher;
use std::cmp::Ord;
use std::collections::HashMap;
use std::hash::BuildHasherDefault;

type FnvHashMap<K, V> = HashMap<K, V, BuildHasherDefault<FnvHasher>>;

type Rect<S> = TypedRect<f32, S>;
type Point<S> = TypedPoint2D<f32, S>;

/// An object that has a bounding box.
///
/// Implementing this trait is not required, but can make insertions easier.
pub trait Spatial<S> {
    /// Returns the boudning box for the object.
    fn aabb(&self) -> Rect<S>;
}

/// An ID unique to a single QuadTree.  This is the object that is
/// returned from queries, and can be used to access the elements stored
/// in the quad tree.
///
/// DO NOT use an ItemId on a quadtree unless the ItemId came from that tree.
#[derive(Eq, PartialEq, Ord, PartialOrd, Hash, Clone, Copy, Debug)]
pub struct ItemId(u32);

#[derive(Debug, Clone)]
struct QuadTreeConfig {
    allow_duplicates: bool,
    max_children: usize,
    min_children: usize,
    max_depth: usize,
    epsilon: f32,
}

/// The main QuadTree structure.  Mainly supports inserting, removing,
/// and querying objects in 3d space.
#[derive(Clone)]
pub struct QuadTree<T, S> {
    root: QuadNode<S>,
    config: QuadTreeConfig,
    id: u32,
    elements: FnvHashMap<ItemId, (T, Rect<S>)>,
}

impl<T: ::std::fmt::Debug, S> ::std::fmt::Debug for QuadTree<T, S> {
    fn fmt(&self, formatter: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {
        formatter
            .debug_struct("QuadTree")
            .field("root", &self.root)
            .field("config", &self.config)
            .field("id", &self.id)
            .field("elements", &self.elements)
            .finish()
    }
}

impl<S> ::std::fmt::Debug for QuadNode<S> {
    fn fmt(&self, formatter: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {
        match self {
            QuadNode::Branch {
                ref aabb,
                ref children,
                ref in_all,
                ref element_count,
                ref depth,
            } => formatter
                .debug_struct("QuadNode")
                .field("aabb", aabb)
                .field("children", children)
                .field("in_all", in_all)
                .field("element_count", element_count)
                .field("depth", depth)
                .finish(),

            QuadNode::Leaf {
                ref aabb,
                ref elements,
                ref depth,
            } => formatter
                .debug_struct("QuadNode")
                .field("aabb", aabb)
                .field("elements", elements)
                .field("depth", depth)
                .finish(),
        }
    }
}

enum QuadNode<S> {
    Branch {
        aabb: Rect<S>,
        element_count: usize,
        depth: usize,
        in_all: Vec<(ItemId, Rect<S>)>,
        children: [(Rect<S>, Box<QuadNode<S>>); 4],
    },
    Leaf {
        aabb: Rect<S>,
        depth: usize,
        elements: Vec<(ItemId, Rect<S>)>,
    },
}

impl<S> Clone for QuadNode<S> {
    fn clone(&self) -> QuadNode<S> {
        match self {
            QuadNode::Branch {
                ref aabb,
                ref children,
                ref in_all,
                ref element_count,
                ref depth,
            } => {
                let children = [
                    children[0].clone(),
                    children[1].clone(),
                    children[2].clone(),
                    children[3].clone(),
                ];
                QuadNode::Branch {
                    aabb: *aabb,
                    children,
                    in_all: in_all.clone(),
                    element_count: *element_count,
                    depth: *depth,
                }
            }
            QuadNode::Leaf {
                ref aabb,
                ref elements,
                ref depth,
            } => QuadNode::Leaf {
                aabb: *aabb,
                elements: elements.clone(),
                depth: *depth,
            },
        }
    }
}

impl<T, S> QuadTree<T, S> {
    /// Constructs a new QuadTree with customizable options.
    ///
    /// * `size`: the enclosing space for the quad-tree.
    /// * `allow_duplicates`: if false, the quadtree will remove objects that have the same bounding box.
    /// * `min_children`: the minimum amount of children that a tree node will have.
    /// * `max_children`: the maximum amount of children that a tree node will have before it gets split.
    /// * `max_depth`: the maximum depth that the tree can grow before it stops.
    pub fn new(
        size: Rect<S>,
        allow_duplicates: bool,
        min_children: usize,
        max_children: usize,
        max_depth: usize,
    ) -> QuadTree<T, S> {
        QuadTree {
            root: QuadNode::Leaf {
                aabb: size,
                elements: Vec::with_capacity(max_children),
                depth: 0,
            },
            config: QuadTreeConfig {
                allow_duplicates,
                max_children,
                min_children,
                max_depth,
                epsilon: 0.0001,
            },
            id: 0,
            elements: HashMap::with_capacity_and_hasher(max_children * 16, Default::default()),
        }
    }

    /// Constructs a new QuadTree with customizable options.
    ///
    /// * `size`: the enclosing space for the quad-tree.
    /// ### Defauts
    /// * `allow_duplicates`: true
    /// * `min_children`: 4
    /// * `max_children`: 16
    /// * `max_depth`: 8
    pub fn default(size: Rect<S>) -> QuadTree<T, S> {
        QuadTree::new(size, true, 4, 16, 8)
    }

    /// Inserts an element with the provided bounding box.
    pub fn insert_with_box(&mut self, t: T, aabb: Rect<S>) -> Option<ItemId> {
        debug_assert!(self.bounding_box().contains(aabb.origin));
        // debug_assert!(self.bounding_box().contains(aabb.max()));

        let &mut QuadTree {
            ref mut root,
            ref config,
            ref mut id,
            ref mut elements,
        } = self;

        let item_id = ItemId(*id);
        *id += 1;

        if root.insert(item_id, aabb, config) {
            elements.insert(item_id, (t, aabb));
            Some(item_id)
        } else {
            None
        }
    }

    /// Returns an ItemId for the first element that was inserted into the tree.
    pub fn first(&self) -> Option<ItemId> {
        self.elements.iter().next().map(|(id, _)| *id)
    }

    /// Inserts an element into the tree.
    pub fn insert(&mut self, t: T) -> Option<ItemId>
    where
        T: Spatial<S>,
    {
        let b = t.aabb();
        self.insert_with_box(t, b)
    }

    /// Retrieves an element by looking it up from the ItemId.
    pub fn get(&self, id: ItemId) -> Option<&T> {
        self.elements.get(&id).map(|&(ref a, _)| a)
    }

    /// Returns an iterator of (element, bounding-box, id) for each element
    /// whose bounding box intersects with `bounding_box`.
    pub fn query(&self, bounding_box: Rect<S>) -> Vec<(&T, &Rect<S>, ItemId)>
    where
        T: ::std::fmt::Debug,
    {
        let mut ids = vec![];
        self.root.query(bounding_box, &self.config, &mut ids);
        ids.sort_by_key(|&(id, _)| id);
        ids.dedup();
        ids.iter()
            .map(|&(id, _)| {
                let &(ref t, ref rect) = match self.elements.get(&id) {
                    Some(e) => e,
                    None => {
                        panic!("looked for {:?}", id);
                    }
                };
                (t, rect, id)
            })
            .collect()
    }

    /// Attempts to remove the item with id `item_id` from the tree.  If that
    /// item was present, it returns a tuple of (element, bounding-box)
    pub fn remove(&mut self, item_id: ItemId) -> Option<(T, Rect<S>)> {
        match self.elements.remove(&item_id) {
            Some((item, aabb)) => {
                self.root.remove(item_id, aabb, &self.config);
                Some((item, aabb))
            }
            None => None,
        }
    }

    /// Returns an iterator over all the items in the tree.
    pub fn iter(&self) -> ::std::collections::hash_map::Iter<ItemId, (T, Rect<S>)> {
        self.elements.iter()
    }

    /// Calls `f` repeatedly for every node in the tree with these arguments
    ///
    /// * `&Rect`: The boudning box of that tree node
    /// * `usize`: The current depth
    /// * `bool`: True if the node is a leaf-node, False if the node is a branch node.
    pub fn inspect<F: FnMut(&Rect<S>, usize, bool)>(&self, mut f: F) {
        self.root.inspect(&mut f);
    }

    /// Returns the number of elements in the tree
    pub fn len(&self) -> usize {
        self.elements.len()
    }

    /// Returns true if the tree is empty.
    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    /// Returns the enclosing bounding-box for the entire tree.
    pub fn bounding_box(&self) -> Rect<S> {
        self.root.bounding_box()
    }
}

impl<S> QuadNode<S> {
    fn bounding_box(&self) -> Rect<S> {
        match self {
            QuadNode::Branch { ref aabb, .. } => *aabb,
            QuadNode::Leaf { ref aabb, .. } => *aabb,
        }
    }

    fn new_leaf(aabb: Rect<S>, depth: usize, config: &QuadTreeConfig) -> QuadNode<S> {
        QuadNode::Leaf {
            aabb,
            elements: Vec::with_capacity(config.max_children / 2),
            depth,
        }
    }

    fn inspect<F: FnMut(&Rect<S>, usize, bool)>(&self, f: &mut F) {
        match self {
            QuadNode::Branch {
                depth,
                ref aabb,
                ref children,
                ..
            } => {
                f(aabb, *depth, false);
                for child in children {
                    child.1.inspect(f);
                }
            }
            QuadNode::Leaf {
                depth, ref aabb, ..
            } => {
                f(aabb, *depth, true);
            }
        }
    }

    fn insert(&mut self, item_id: ItemId, item_aabb: Rect<S>, config: &QuadTreeConfig) -> bool {
        let mut into = None;
        let mut did_insert = false;
        match *self {
            QuadNode::Branch {
                ref aabb,
                ref mut in_all,
                ref mut children,
                ref mut element_count,
                ..
            } => {
                if item_aabb.contains(midpoint(*aabb)) {
                    // Only insert if there isn't another item with a very
                    // similar aabb.
                    if config.allow_duplicates
                        || !in_all
                            .iter()
                            .any(|&(_, e_bb)| close_to_rect(e_bb, item_aabb, config.epsilon))
                    {
                        in_all.push((item_id, item_aabb));
                        did_insert = true;
                        *element_count += 1;
                    }
                } else {
                    for &mut (aabb, ref mut child) in children {
                        if (my_intersects(aabb, item_aabb)
                            || close_to_rect(aabb, item_aabb, config.epsilon))
                            && child.insert(item_id, item_aabb, config)
                        {
                            *element_count += 1;
                            did_insert = true;
                        }
                    }
                }
            }

            QuadNode::Leaf {
                aabb,
                ref mut elements,
                ref depth,
            } => {
                if elements.len() == config.max_children && *depth != config.max_depth {
                    // STEAL ALL THE CHILDREN MUAHAHAHAHA
                    let mut extracted_children = Vec::new();
                    ::std::mem::swap(&mut extracted_children, elements);
                    extracted_children.push((item_id, item_aabb));
                    did_insert = true;

                    let split = split_quad(aabb);
                    into = Some((
                        extracted_children,
                        QuadNode::Branch {
                            aabb,
                            in_all: Vec::new(),
                            children: [
                                (
                                    split[0],
                                    Box::new(QuadNode::new_leaf(split[0], depth + 1, config)),
                                ),
                                (
                                    split[1],
                                    Box::new(QuadNode::new_leaf(split[1], depth + 1, config)),
                                ),
                                (
                                    split[2],
                                    Box::new(QuadNode::new_leaf(split[2], depth + 1, config)),
                                ),
                                (
                                    split[3],
                                    Box::new(QuadNode::new_leaf(split[3], depth + 1, config)),
                                ),
                            ],
                            element_count: 0,
                            depth: *depth,
                        },
                    ));
                } else if config.allow_duplicates
                    || !elements
                        .iter()
                        .any(|&(_, e_bb)| close_to_rect(e_bb, item_aabb, config.epsilon))
                {
                    elements.push((item_id, item_aabb));
                    did_insert = true;
                }
            }
        }

        // If we transitioned from a leaf node to a branch node, we
        // need to update ourself and re-add all the children that
        // we used to have
        // in our this leaf into our new leaves.
        if let Some((extracted_children, new_node)) = into {
            *self = new_node;
            for (child_id, child_aabb) in extracted_children {
                self.insert(child_id, child_aabb, config);
            }
        }

        did_insert
    }

    fn remove(&mut self, item_id: ItemId, item_aabb: Rect<S>, config: &QuadTreeConfig) -> bool {
        fn remove_from<S>(v: &mut Vec<(ItemId, Rect<S>)>, item: ItemId) -> bool {
            if let Some(index) = v.iter().position(|a| a.0 == item) {
                v.swap_remove(index);
                true
            } else {
                false
            }
        }

        let mut compact = None;
        let removed = match *self {
            QuadNode::Branch {
                ref depth,
                ref aabb,
                ref mut in_all,
                ref mut children,
                ref mut element_count,
                ..
            } => {
                let mut did_remove = false;

                if item_aabb.contains(midpoint(*aabb)) {
                    did_remove = remove_from(in_all, item_id);
                } else {
                    for &mut (child_aabb, ref mut child_tree) in children {
                        if my_intersects(child_aabb, item_aabb)
                            || close_to_rect(child_aabb, item_aabb, config.epsilon)
                        {
                            did_remove |= child_tree.remove(item_id, item_aabb, config);
                        }
                    }
                }

                if did_remove {
                    *element_count -= 1;
                    if *element_count < config.min_children {
                        compact = Some((*element_count, *aabb, *depth));
                    }
                }
                did_remove
            }

            QuadNode::Leaf {
                ref mut elements, ..
            } => remove_from(elements, item_id),
        };

        if let Some((size, aabb, depth)) = compact {
            let mut elements = Vec::with_capacity(size);
            self.query(aabb, config, &mut elements);
            elements.sort_by(|&(id1, _), &(ref id2, _)| id1.cmp(id2));
            elements.dedup();
            *self = QuadNode::Leaf {
                aabb,
                elements,
                depth,
            };
        }
        removed
    }

    fn query(
        &self,
        query_aabb: Rect<S>,
        config: &QuadTreeConfig,
        out: &mut Vec<(ItemId, Rect<S>)>,
    ) {
        fn match_all<S>(
            elements: &Vec<(ItemId, Rect<S>)>,
            query_aabb: Rect<S>,
            out: &mut Vec<(ItemId, Rect<S>)>,
            config: &QuadTreeConfig,
        ) {
            for &(ref child_id, child_aabb) in elements {
                if my_intersects(query_aabb, child_aabb)
                    || close_to_rect(query_aabb, child_aabb, config.epsilon)
                {
                    out.push((*child_id, child_aabb))
                }
            }
        }

        match *self {
            QuadNode::Branch {
                ref in_all,
                ref children,
                ..
            } => {
                match_all(in_all, query_aabb, out, config);

                for &(child_aabb, ref child_tree) in children {
                    if my_intersects(query_aabb, child_aabb) {
                        child_tree.query(query_aabb, config, out);
                    }
                }
            }
            QuadNode::Leaf { ref elements, .. } => match_all(elements, query_aabb, out, config),
        }
    }
}

impl<S> Spatial<S> for Rect<S> {
    fn aabb(&self) -> Rect<S> {
        *self
    }
}

impl<S> Spatial<S> for Point<S> {
    fn aabb(&self) -> Rect<S> {
        Rect::new(*self, TypedSize2D::new(0.0, 0.0))
    }
}

fn midpoint<S>(rect: Rect<S>) -> Point<S> {
    let origin = rect.origin;
    let half = rect.size.to_vector() / 2.0;
    origin + half
}

fn my_intersects<S>(a: Rect<S>, b: Rect<S>) -> bool {
    a.intersects(&b)
        || a.min_x() == b.min_x()
        || a.min_y() == b.min_y()
        || a.max_x() == b.max_x()
        || a.max_y() == b.max_y()
}

fn split_quad<S>(rect: Rect<S>) -> [Rect<S>; 4] {
    use euclid::vec2;
    let origin = rect.origin;
    let half = rect.size / 2.0;

    [
        Rect::new(origin, half),
        Rect::new(origin + vec2(half.width, 0.0), half),
        Rect::new(origin + vec2(0.0, half.height), half),
        Rect::new(origin + vec2(half.width, half.height), half),
    ]
}

fn close_to_point<S>(a: Point<S>, b: Point<S>, epsilon: f32) -> bool {
    (a.x - b.x).abs() < epsilon && (a.y - b.y).abs() < epsilon
}
fn close_to_rect<S>(a: Rect<S>, b: Rect<S>, epsilon: f32) -> bool {
    close_to_point(a.origin, b.origin, epsilon) && close_to_point(a.max(), b.max(), epsilon)
}

#[test]
fn test_boundary_conditions() {
    use euclid::*;

    let total: Rect<f32, f32> = Rect::new(point2(0.0, 0.0), vec2(10.0, 10.0).to_size());
    let quads = split_quad(total);
    let config = QuadTreeConfig {
        allow_duplicates: true,
        max_children: 200,
        min_children: 0,
        max_depth: 5,
        epsilon: 0.001,
    };

    let mut branch = QuadNode::Branch {
        aabb: total,
        in_all: vec![],
        element_count: 0,
        depth: 1,
        children: [
            (
                quads[0],
                Box::new(QuadNode::Leaf {
                    aabb: quads[0],
                    elements: vec![],
                    depth: 2,
                }),
            ),
            (
                quads[1],
                Box::new(QuadNode::Leaf {
                    aabb: quads[1],
                    elements: vec![],
                    depth: 2,
                }),
            ),
            (
                quads[2],
                Box::new(QuadNode::Leaf {
                    aabb: quads[2],
                    elements: vec![],
                    depth: 2,
                }),
            ),
            (
                quads[3],
                Box::new(QuadNode::Leaf {
                    aabb: quads[3],
                    elements: vec![],
                    depth: 2,
                }),
            ),
        ],
    };

    // Top left corner
    assert!(branch.insert(
        ItemId(0),
        Rect::new(point2(0.0, 0.0), vec2(0.0, 0.0).to_size()),
        &config
    ));
    // Middle
    assert!(branch.insert(
        ItemId(0),
        Rect::new(point2(5.0, 5.0), vec2(0.0, 0.0).to_size()),
        &config
    ));
}
