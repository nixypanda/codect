{-# LANGUAGE GADTs #-}

module Data.Header
  ( User(..),
    mkUser,
  )
where

import Data.Text (Text)

mkUser :: Text -> User
mkUser name = User name
