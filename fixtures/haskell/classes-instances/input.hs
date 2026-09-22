module Classes where

class Eq a => Container f a where
  empty :: f a
  insert :: a -> f a -> f a
  toList :: f a -> [a]
  size :: f a -> Int
  size _ = 0

instance Container [] Int where
  empty = []
  insert x xs = x : xs
  toList = id
  size = length
