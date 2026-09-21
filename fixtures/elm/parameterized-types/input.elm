module Container exposing (Tree(..))


type Tree a b
    = Leaf
    | Node (Tree a b) a b
