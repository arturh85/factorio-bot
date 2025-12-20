#![deny(missing_docs)]

//! A simple spacial partitioning data structure that allows fast queries for
//! 2-dimensional objects.
//!
//! As the name implies, the tree is a mapping from axis-aligned-bounding-box => object.

use euclid::{Point2D, Rect as EuclidRect, Size2D};
use fnv::FnvHashMap;
use smallvec::{Array, SmallVec};
use std::cmp::Ord;

type Rect<S> = EuclidRect<f32, S>;
type Point<S> = Point2D<f32, S>;

/// An object that has a bounding box.
///
/// Implementing this trait is not required, but can make
/// insertions easier.
pub trait Spatial<S> {
    /// Returns the boudning box for the object.
    fn aabb(&self) -> Rect<S>;
}

/// Used to determine if a query should keep going or not
pub type QueryResult<B> = Result<(), B>;

/// An ID unique to a single QuadTree.  This is the object
/// that is returned from queries, and can be used to
/// access the elements stored in the quad tree.
///
/// DO NOT use an ItemId on a quadtree unless the ItemId
/// came from that tree.
#[derive(
    Eq, PartialEq, Ord, PartialOrd, Hash, Clone, Copy, Debug, Serialize, Deserialize, Default,
)]
pub struct ItemId(u32);

#[derive(Debug, Clone, Serialize, Deserialize)]
struct QuadTreeConfig {
    allow_duplicates: bool,
    max_children: usize,
    min_children: usize,
    max_depth: usize,
    epsilon: f32,
}

/// The main QuadTree structure.  Mainly supports
/// inserting, removing, and querying objects in 3d space.
#[derive(Clone, Serialize, Deserialize)]
pub struct QuadTree<T, S, A: Array<Item = (ItemId, Rect<S>)>> {
    root: QuadNode<S, A>,
    config: QuadTreeConfig,
    id: u32,
    elements: FnvHashMap<ItemId, (T, Rect<S>)>,
}

#[derive(Serialize, Deserialize)]
#[allow(clippy::type_complexity)]
enum QuadNode<S, A: Array<Item = (ItemId, Rect<S>)>> {
    Branch {
        aabb: Rect<S>,
        element_count: usize,
        depth: usize,
        in_all: SmallVec<A>,
        children: [(Rect<S>, Box<QuadNode<S, A>>); 4],
    },
    Leaf {
        aabb: Rect<S>,
        depth: usize,
        elements: SmallVec<A>,
    },
}

impl<S, A: Array<Item = (ItemId, Rect<S>)>> Clone for QuadNode<S, A> {
    fn clone(&self) -> QuadNode<S, A> {
        match self {
            QuadNode::Branch {
                aabb,
                children,
                in_all,
                element_count,
                depth,
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
                aabb,
                elements,
                depth,
            } => QuadNode::Leaf {
                aabb: *aabb,
                elements: elements.clone(),
                depth: *depth,
            },
        }
    }
}

impl<T, S, A: Array<Item = (ItemId, Rect<S>)>> QuadTree<T, S, A> {
    /// Constructs a new QuadTree with customizable options.
    ///
    /// * `size`: the enclosing space for the quad-tree.
    /// * `allow_duplicates`: if false, the quadtree will
    /// remove objects that have the same bounding box.
    /// * `min_children`: the minimum amount of children
    /// that a tree node will have. * `max_children`:
    /// the maximum amount of children that a tree node
    /// will have before it gets split. * `max_depth`:
    /// the maximum depth that the tree can grow before it
    /// stops.
    pub fn new(
        size: Rect<S>,
        allow_duplicates: bool,
        min_children: usize,
        max_children: usize,
        max_depth: usize,
        size_hint: usize,
    ) -> QuadTree<T, S, A> {
        QuadTree {
            root: QuadNode::Leaf {
                aabb: size,
                elements: SmallVec::new(),
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
            elements: std::collections::HashMap::with_capacity_and_hasher(
                size_hint,
                Default::default(),
            ),
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
    pub fn default(size: Rect<S>, size_hint: usize) -> QuadTree<T, S, A> {
        QuadTree::new(size, true, 4, 16, 8, size_hint)
    }

    /// Inserts an element with the provided bounding box.
    pub fn insert_with_box(&mut self, t: T, aabb: Rect<S>) -> Option<ItemId> {
        debug_assert!(self.bounding_box().contains(aabb.origin));
        debug_assert!(self.bounding_box().contains(aabb.max()));

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

    /// Returns an ItemId for the first element that was
    /// inserted into the tree.
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

    /// Retrieves an element by looking it up from the
    /// ItemId.
    pub fn get(&self, id: ItemId) -> Option<&T> {
        self.elements.get(&id).map(|&(ref a, _)| a)
    }

    /// Returns an iterator of (element, bounding-box, id)
    /// for each element whose bounding box intersects
    /// with `bounding_box`.
    pub fn query(&self, bounding_box: Rect<S>) -> SmallVec<[(&T, Rect<S>, ItemId); 3]>
    where
        T: ::std::fmt::Debug,
    {
        let mut out: SmallVec<[(_, _, _); 3]> = Default::default();
        let _ = self
            .root
            .query::<(), _>(bounding_box, &self.config, &mut |id, bb| {
                out.push((&self.elements.get(&id).unwrap().0, bb, id));
                Ok(())
            });
        out.sort_by_key(|&(_, _, id)| id);
        out.dedup_by_key(|&mut (_, _, id)| id);
        out
    }

    /// Executes 'on_find' for every element found in the
    /// bounding-box
    #[allow(clippy::needless_lifetimes)]
    pub fn custom_query<'a, B, F>(&'a self, query_aabb: Rect<S>, on_find: &mut F) -> QueryResult<B>
    where
        F: FnMut(ItemId, Rect<S>) -> QueryResult<B>,
    {
        self.root.query(query_aabb, &self.config, on_find)
    }

    /// Attempts to remove the item with id `item_id` from
    /// the tree.  If that item was present, it returns
    /// a tuple of (element, bounding-box)
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
    pub fn iter(&self) -> ::std::collections::hash_map::Iter<'_, ItemId, (T, Rect<S>)> {
        self.elements.iter()
    }

    /// Calls `f` repeatedly for every node in the tree
    /// with these arguments
    ///
    /// * `&Rect`: The bounding box of that tree node
    /// * `usize`: The current depth
    /// * `bool`: True if the node is a leaf-node, False if
    /// the node is a branch node.
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

    /// Returns the enclosing bounding-box for the entire
    /// tree.
    pub fn bounding_box(&self) -> Rect<S> {
        self.root.bounding_box()
    }
}

impl<S, A: Array<Item = (ItemId, Rect<S>)>> QuadNode<S, A> {
    fn bounding_box(&self) -> Rect<S> {
        match self {
            QuadNode::Branch { aabb, .. } => *aabb,
            QuadNode::Leaf { aabb, .. } => *aabb,
        }
    }

    fn new_leaf(aabb: Rect<S>, depth: usize) -> QuadNode<S, A> {
        QuadNode::Leaf {
            aabb,
            elements: SmallVec::new(),
            depth,
        }
    }

    fn inspect<F: FnMut(&Rect<S>, usize, bool)>(&self, f: &mut F) {
        match *self {
            QuadNode::Branch {
                depth,
                ref aabb,
                ref children,
                ..
            } => {
                f(aabb, depth, false);
                for child in children {
                    child.1.inspect(f);
                }
            }
            QuadNode::Leaf {
                depth, ref aabb, ..
            } => {
                f(aabb, depth, true);
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
                    let mut extracted_children = SmallVec::new();
                    ::std::mem::swap(&mut extracted_children, elements);
                    extracted_children.push((item_id, item_aabb));
                    did_insert = true;

                    let split = split_quad(aabb);
                    into = Some((
                        extracted_children,
                        QuadNode::Branch {
                            aabb,
                            in_all: SmallVec::new(),
                            children: [
                                (split[0], Box::new(QuadNode::new_leaf(split[0], depth + 1))),
                                (split[1], Box::new(QuadNode::new_leaf(split[1], depth + 1))),
                                (split[2], Box::new(QuadNode::new_leaf(split[2], depth + 1))),
                                (split[3], Box::new(QuadNode::new_leaf(split[3], depth + 1))),
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
        if !did_insert {
            panic!(
                "didn't insert {:?} into {:?}",
                item_aabb,
                self.bounding_box()
            );
        }
        did_insert
    }

    fn remove(&mut self, item_id: ItemId, item_aabb: Rect<S>, config: &QuadTreeConfig) -> bool {
        fn remove_from<S, A: Array<Item = (ItemId, Rect<S>)>>(
            v: &mut SmallVec<A>,
            item: ItemId,
        ) -> bool {
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
            let mut elements: SmallVec<A> = SmallVec::with_capacity(size);
            self.query::<(), _>(aabb, config, &mut |id, bb| {
                elements.push((id, bb));
                Ok(())
            })
            .ok();
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

    fn query<B, F>(
        &self,
        query_aabb: Rect<S>,
        config: &QuadTreeConfig,
        on_find: &mut F,
    ) -> QueryResult<B>
    where
        F: FnMut(ItemId, Rect<S>) -> QueryResult<B>,
    {
        fn match_all<B, S, F, A: Array<Item = (ItemId, Rect<S>)>>(
            elements: &SmallVec<A>,
            query_aabb: Rect<S>,
            on_find: &mut F,
            config: &QuadTreeConfig,
        ) -> QueryResult<B>
        where
            F: FnMut(ItemId, Rect<S>) -> QueryResult<B>,
        {
            for &(child_id, child_aabb) in elements {
                if my_intersects(query_aabb, child_aabb)
                    || close_to_rect(query_aabb, child_aabb, config.epsilon)
                {
                    on_find(child_id, child_aabb)?;
                }
            }
            Ok(())
        }

        match *self {
            QuadNode::Branch {
                ref in_all,
                ref children,
                ..
            } => {
                match_all(in_all, query_aabb, on_find, config)?;

                for &(child_aabb, ref child_tree) in children {
                    if my_intersects(query_aabb, child_aabb) {
                        child_tree.query(query_aabb, config, on_find)?;
                    }
                }
            }
            QuadNode::Leaf { ref elements, .. } => {
                match_all(elements, query_aabb, on_find, config)?
            }
        }
        Ok(())
    }
}

impl<S> Spatial<S> for Rect<S> {
    fn aabb(&self) -> Rect<S> {
        *self
    }
}

impl<S> Spatial<S> for Point<S> {
    fn aabb(&self) -> Rect<S> {
        Rect::new(*self, Size2D::new(0.0, 0.0))
    }
}

fn midpoint<S>(rect: Rect<S>) -> Point<S> {
    let origin = rect.origin;
    let half = rect.size.to_vector() / 2.0;
    origin + half
}

fn my_intersects<S>(a: Rect<S>, b: Rect<S>) -> bool {
    a.intersects(&b) || a.contains(b.origin) || a.contains(b.max())
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
fn weird_case() {
    use euclid::*;
    let bb: Rect<f32, f32> = Rect::new(point2(0.0, 0.0), vec2(10.0, 10.0).to_size());
    let query = Rect::new(point2(20.0, 0.0), vec2(1.0, 0.0).to_size());
    assert!(!my_intersects(bb, query));
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

    let mut branch: QuadNode<_, [(ItemId, Rect<_, _>); 32]> = QuadNode::Branch {
        aabb: total,
        in_all: SmallVec::new(),
        element_count: 0,
        depth: 1,
        children: [
            (
                quads[0],
                Box::new(QuadNode::Leaf {
                    aabb: quads[0],
                    elements: SmallVec::new(),
                    depth: 2,
                }),
            ),
            (
                quads[1],
                Box::new(QuadNode::Leaf {
                    aabb: quads[1],
                    elements: SmallVec::new(),
                    depth: 2,
                }),
            ),
            (
                quads[2],
                Box::new(QuadNode::Leaf {
                    aabb: quads[2],
                    elements: SmallVec::new(),
                    depth: 2,
                }),
            ),
            (
                quads[3],
                Box::new(QuadNode::Leaf {
                    aabb: quads[3],
                    elements: SmallVec::new(),
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

impl<T: ::std::fmt::Debug, S, A: Array<Item = (ItemId, Rect<S>)>> ::std::fmt::Debug
    for QuadTree<T, S, A>
{
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

impl<S, A: Array<Item = (ItemId, Rect<S>)>> ::std::fmt::Debug for QuadNode<S, A> {
    fn fmt(&self, formatter: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {
        match *self {
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
