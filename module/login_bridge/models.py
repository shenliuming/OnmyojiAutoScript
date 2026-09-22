from typing import Optional

from pydantic import BaseModel


class LoginDetectRequest(BaseModel):
    config_name: str


class LoginDetectResponse(BaseModel):
    ready: bool
    ambiguous: bool = False
    message: str
    masked_account: Optional[str] = None
    character_name: Optional[str] = None
    server_name: Optional[str] = None
    game_uid: Optional[str] = None
