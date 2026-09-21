module Nested exposing (..)


outer : Int
outer =
    let
        inner x =
            x + 1

        helper : Int -> Int
        helper y =
            y
    in
    inner 1
