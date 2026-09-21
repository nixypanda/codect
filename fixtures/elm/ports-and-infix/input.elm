port module Ports exposing (..)


port receive : (String -> msg) -> Sub msg


infix right 5 (</>) = combine


combine : List a -> List a -> List a
combine left right =
    left
