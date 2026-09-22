import asyncio

from fastapi import APIRouter

from module.foster_bridge.models import FosterExecuteRequest, FosterExecuteResponse
from module.foster_bridge.service import FosterBridgeService


foster_app = APIRouter()


@foster_app.post("/foster/execute", response_model=FosterExecuteResponse)
async def execute_foster(request: FosterExecuteRequest) -> FosterExecuteResponse:
    service = FosterBridgeService()
    return await asyncio.to_thread(service.execute, request)
