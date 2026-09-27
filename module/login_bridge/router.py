import asyncio

from fastapi import APIRouter

from module.login_bridge.models import (
    LoginDetectRequest,
    LoginDetectResponse,
    LoginPlatformRequest,
    LoginSelectIdentityRequest,
)
from module.login_bridge.service import LoginDetectService


login_bridge_app = APIRouter()


@login_bridge_app.post("/login/detect", response_model=LoginDetectResponse)
async def detect_login(request: LoginDetectRequest) -> LoginDetectResponse:
    service = LoginDetectService()
    return await asyncio.to_thread(service.detect, request.config_name)


@login_bridge_app.post("/login/platform", response_model=LoginDetectResponse)
async def select_login_platform(request: LoginPlatformRequest) -> LoginDetectResponse:
    service = LoginDetectService()
    return await asyncio.to_thread(
        service.select_platform,
        request.config_name,
        request.platform,
    )


@login_bridge_app.post("/login/select-identity", response_model=LoginDetectResponse)
async def select_login_identity(request: LoginSelectIdentityRequest) -> LoginDetectResponse:
    service = LoginDetectService()
    return await asyncio.to_thread(
        service.select_identity,
        request.config_name,
        request.server_name,
        request.character_name,
    )
