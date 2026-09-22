module Foreign where

foreign import ccall "math.h sin" c_sin :: Double -> Double

foreign import ccall unsafe "stdlib.h malloc"
  c_malloc :: Int -> IO (Ptr a)

pattern Single :: a -> [a]
pattern Single x = [x]
