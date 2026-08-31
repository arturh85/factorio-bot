import type {ScriptTreeNode} from '@/api/types';

/**
 * Return a copy of `nodes` in which the node identified by `key` has `children`.
 *
 * Directory listings arrive one directory at a time (`GET /api/v1/scripts`),
 * so an expanded directory's children have to be spliced into a tree that is
 * already on screen. Matching on `key` -- the node's full path -- rather than
 * walking the path's segments by label means a directory whose name repeats at
 * another depth cannot be confused for its namesake.
 *
 * Pure on purpose: the caller assigns the result, so Vue sees one replacement
 * rather than a mutation it has to detect.
 */
export function mergeNodes(
    nodes: ScriptTreeNode[],
    key: string,
    children: ScriptTreeNode[]
): ScriptTreeNode[] {
    return nodes.map(node => {
        if (node.key === key) {
            return {...node, children};
        }
        if (node.children.length === 0) {
            return node;
        }
        return {...node, children: mergeNodes(node.children, key, children)};
    });
}
