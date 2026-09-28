module Wrapping exposing (..)


create :
    UserId
    -> Profile
    -> Permissions
    -> Organization
    -> AccountSettings
    -> BillingProfile
    -> Result Error User
create id profile =
    User id profile
