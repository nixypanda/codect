module Wrapping where

combine :: String -> Int -> [String] -> OrganizationId -> AccountSettings -> BillingProfile -> IO (Either Error User)
combine name count xs org settings profile = undefined

render :: (Monad m, Show a) => a -> OrganizationId -> AccountSettings -> BillingProfile -> m (Either Error User)
render value org settings profile = undefined

journalWithHistoricalCostsUsing :: (Day -> Hledger.MixedAmount -> Hledger.MixedAmount)

scopeScanFixtures :: [(String, Text.Text, AssetClassMappings, InvestmentMappings, [Maybe Text.Text])]

genSummaryScenario :: Gen (Text.Text, AssetClassMappings, InvestmentMappings, Maybe Text.Text)
