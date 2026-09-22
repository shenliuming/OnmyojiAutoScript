import asyncio

from fastapi import APIRouter

from module.login_bridge.models import LoginDetectRequest, LoginDetectResponse
from module.login_bridge.service import LoginDetectService


login_bridge_app = APIRouter()


@login_bridge_app.post("/login/detect", response_model=LoginDetectResponse)
async def detect_login(request: LoginDetectRequest) -> LoginDetectResponse:
    service = LoginDetectService()
    return await asyncio.to_thread(service.detect, request.config_name)
