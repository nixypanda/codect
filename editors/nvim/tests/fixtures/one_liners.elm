port module OneLiners exposing (..)


port receive : (String -> msg) -> Sub msg


infix right 5 (</>) = combine
