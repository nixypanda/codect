-- leading comment
module Formatting where

import Data.Text (Text)

-- | A Haddock comment.
foo :: Int -> Int
foo   x   =   x + 1

data Maybe a = Nothing | Just a
  deriving ( Eq , Show )
