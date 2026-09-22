from typing import List, Optional

from pydantic import BaseModel, Field


class FosterStageCheckpoint(BaseModel):
    stage: str
    occurred_at: str


class FosterExecuteRequest(BaseModel):
    config_name: str
    job_id: int
    resource_mode: str = "USER_FRIEND"
    resource_type: Optional[str] = None
    provider_alias: Optional[str] = None
    masked_account: Optional[str] = None
    account_aliases: List[str] = Field(default_factory=list)
    character_name: Optional[str] = None
    server_name: Optional[str] = None
    game_uid: Optional[str] = None


class FosterDetectedIdentity(BaseModel):
    masked_account: Optional[str] = None
    character_name: Optional[str] = None
    server_name: Optional[str] = None
    game_uid: Optional[str] = None


class FosterExecuteResponse(BaseModel):
    success: bool
    code: str
    message: str
    remaining_seconds: Optional[int] = None
    screenshot_url: Optional[str] = None
    stages: List[FosterStageCheckpoint] = Field(default_factory=list)
    detected_identity: FosterDetectedIdentity = Field(default_factory=FosterDetectedIdentity)
