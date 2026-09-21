{- block comment before the module -}
module   Commented
    exposing ( .. )

-- a line comment
import   Html


{-| A documented alias. -}
type alias   Thing =
    { one   :   String -- trailing field comment
    , two :Int
    }


{- doc for f -}
f    x =
    x -- body comment
