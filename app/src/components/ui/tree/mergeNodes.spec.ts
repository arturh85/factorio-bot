import {describe, expect, it} from 'vitest';
import type {ScriptTreeNode} from '@/api/types';
import {mergeNodes} from './mergeNodes';

const leaf = (key: string, label: string): ScriptTreeNode => ({key, label, leaf: true, children: []});
const dir = (key: string, label: string, children: ScriptTreeNode[] = []): ScriptTreeNode =>
    ({key, label, leaf: false, children});

describe('mergeNodes', () => {
    it('fills in the children of a top-level directory', () => {
        const tree = [dir('/sub', 'sub'), leaf('/a.lua', 'a.lua')];
        const merged = mergeNodes(tree, '/sub', [leaf('/sub/b.lua', 'b.lua')]);
        expect(merged[0].children.map(node => node.key)).toEqual(['/sub/b.lua']);
    });

    it('fills in the children of a nested directory without disturbing its siblings', () => {
        const tree = [
            dir('/sub', 'sub', [dir('/sub/deep', 'deep'), leaf('/sub/b.lua', 'b.lua')]),
            leaf('/a.lua', 'a.lua')
        ];
        const merged = mergeNodes(tree, '/sub/deep', [leaf('/sub/deep/c.lua', 'c.lua')]);

        // A top-level-only implementation leaves this empty.
        expect(merged[0].children[0].children.map(node => node.key)).toEqual(['/sub/deep/c.lua']);
        expect(merged[0].children[1].key).toBe('/sub/b.lua');
        expect(merged[1].key).toBe('/a.lua');
    });

    it('leaves the tree unchanged when the key is not in it', () => {
        const tree = [dir('/sub', 'sub')];
        const merged = mergeNodes(tree, '/nope', [leaf('/nope/x.lua', 'x.lua')]);
        expect(merged[0].children).toEqual([]);
    });

    it('does not mutate the input', () => {
        const tree = [dir('/sub', 'sub')];
        mergeNodes(tree, '/sub', [leaf('/sub/b.lua', 'b.lua')]);
        expect(tree[0].children).toEqual([]);
    });

    // The discriminating case: two siblings sharing a label, one nested (not
    // at the root). The old `ScriptTree.vue` merge walked `new_key`'s path
    // segments matching each one against `label`, so at the last segment
    // `pointer.children.find(n => n.label === 'shared')` returns whichever
    // same-labelled sibling comes first in the array -- here the decoy, not
    // the real target -- and mutates that one instead. Matching on the
    // node's own unique `key` cannot make that mistake: only one node in the
    // whole tree has key '/root1/shared'.
    it('updates the node identified by key, not the first sibling sharing its label', () => {
        const tree = [
            dir('/root1', 'root1', [
                dir('/root1/decoy', 'shared', [leaf('/root1/decoy/WRONG.lua', 'WRONG.lua')]),
                dir('/root1/shared', 'shared', [])
            ])
        ];
        const merged = mergeNodes(tree, '/root1/shared', [leaf('/root1/shared/right.lua', 'right.lua')]);

        expect(merged[0].children[1].children.map(node => node.key)).toEqual(['/root1/shared/right.lua']);
        expect(merged[0].children[0].children.map(node => node.key)).toEqual(['/root1/decoy/WRONG.lua']);
    });
});
