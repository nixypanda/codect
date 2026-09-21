module User exposing (..)


create : UserId -> Profile -> User
create id profile =
    User id profile


normalize x =
    x
