module App where

foo :: Int -> Int
foo x = x + 1

bar x = x

baz, qux :: Int
baz = 1
qux = 2

compute :: forall a. a -> a
compute value = value
