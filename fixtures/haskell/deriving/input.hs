module Deriving where

data Color = Red | Green | Blue
  deriving stock (Eq, Show)

newtype Meters = Meters Double
  deriving newtype (Num, Eq)

data Wrapped = Wrapped Int
  deriving (Eq) via ComparingWrapper
