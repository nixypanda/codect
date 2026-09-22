module Records where

data User = User
  { name :: Text,
    email :: Email,
    age :: Int
  }
  deriving (Eq, Show)

newtype Wrap = Wrap
  { unWrap :: Int
  }
  deriving newtype (Num)
