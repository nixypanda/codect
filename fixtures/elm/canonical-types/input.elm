module User exposing (User(..), Profile)


type User
    = User UserId Profile


type alias Profile =
    { name : String
    , email : Email
    }
