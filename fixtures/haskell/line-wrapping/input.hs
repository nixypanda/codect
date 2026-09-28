module Wrapping where

combine :: String -> Int -> [String] -> OrganizationId -> AccountSettings -> BillingProfile -> IO (Either Error User)
combine name count xs org settings profile = undefined

render :: (Monad m, Show a) => a -> OrganizationId -> AccountSettings -> BillingProfile -> m (Either Error User)
render value org settings profile = undefined
