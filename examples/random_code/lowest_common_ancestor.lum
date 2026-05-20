# EXPECT:
# lca 3
# found p 5
# found q 1
# missing true

record TreeNode {
    val Int
    left Option[TreeNode]
    right Option[TreeNode]
}

def chooseNode(left Option[TreeNode], right Option[TreeNode]) Option[TreeNode] =
    if !left.isEmpty() then left else right

def find(
    node Option[TreeNode],
    valP Int,
    valQ Int,
    foundPNode Option[TreeNode],
    foundQNode Option[TreeNode]
) { foundP Option[TreeNode], foundQ Option[TreeNode], ancestor Option[TreeNode] } = {

    unwrap current <- node else {
        record { foundP = foundPNode, foundQ = foundQNode, ancestor = None() }
    }

    var currentFoundP = foundPNode
    var currentFoundQ = foundQNode

    if currentFoundP.isEmpty() && current.val == valP {
        currentFoundP := Some(current)
    }

    if currentFoundQ.isEmpty() && current.val == valQ then currentFoundQ := Some(current)

    foundPCount = currentFoundP.map(_ -> 1).getOrElse(0)
    foundQCount = currentFoundQ.map(_ -> 1).getOrElse(0)

    if foundPCount + foundQCount < 2 {
        leftResult = find(current.left, valP, valQ, currentFoundP, currentFoundQ)
        rightResult = find(current.right, valP, valQ, currentFoundP, currentFoundQ)

        var mergedAncestor = chooseNode(leftResult.ancestor, rightResult.ancestor)
        mergedP = chooseNode(leftResult.foundP, rightResult.foundP)
        mergedQ = chooseNode(leftResult.foundQ, rightResult.foundQ)

        if !mergedP.isEmpty() && !mergedQ.isEmpty() && mergedAncestor.isEmpty() && foundPNode.isEmpty() && foundQNode.isEmpty() {
            mergedAncestor := Some(current)
        }

        return record(mergedP, mergedQ, mergedAncestor)
    }

    record(currentFoundP, currentFoundQ, None())
}

def lowestCommonAncestor(root TreeNode, valP Int, valQ Int) Option[TreeNode] =
    find(Some(root), valP, valQ, None(), None()).ancestor

def main() Unit {
    root = TreeNode(
        3,
        Some(TreeNode(
            5,
            Some(TreeNode(6, None(), None())),
            Some(TreeNode(
                2,
                Some(TreeNode(7, None(), None())),
                Some(TreeNode(4, None(), None()))
            ))
        )),
        Some(TreeNode(
            1,
            Some(TreeNode(0, None(), None())),
            Some(TreeNode(8, None(), None()))
        ))
    )

    result = find(Some(root), 5, 1, None(), None())
    missing = lowestCommonAncestor(root, 10, 42)

    unwrap ancestor <- result.ancestor else OS.panic("missing ancestor")
    unwrap foundP <- result.foundP else OS.panic("missing foundP")
    unwrap foundQ <- result.foundQ else OS.panic("missing foundQ")
    OS.println("lca ${ancestor.val}")
    OS.println("found p ${foundP.val}")
    OS.println("found q ${foundQ.val}")
    OS.println("missing ${missing.isEmpty()}")
}
