from datetime import datetime, timezone

from module.config.config import Config
from module.device.device import Device
from tasks.Component.SwitchAccount.switch_account import SwitchAccount
from tasks.Component.SwitchAccount.switch_account_config import AccountInfo
from tasks.KekkaiUtilize.foster_once import FosterOnceTask

from module.foster_bridge.models import (
    FosterDetectedIdentity,
    FosterExecuteRequest,
    FosterExecuteResponse,
    FosterStageCheckpoint,
)


def _checkpoint(stage: str) -> FosterStageCheckpoint:
    return FosterStageCheckpoint(
        stage=stage,
        occurred_at=datetime.now(timezone.utc).isoformat(),
    )


class FosterBridgeService:
    def execute(self, request: FosterExecuteRequest) -> FosterExecuteResponse:
        if request.resource_mode != "USER_FRIEND":
            return FosterExecuteResponse(
                success=False,
                code="PROVIDER_NOT_FOUND",
                message="PLATFORM foster execution is not enabled before resource allocation phase",
            )

        account_hint = request.masked_account
        aliases = [value for value in request.account_aliases if value]
        if not account_hint and aliases:
            account_hint = aliases[0]

        if not account_hint or not request.character_name:
            return FosterExecuteResponse(
                success=False,
                code="IDENTITY_MISMATCH",
                message="missing account or character identity hints",
            )

        stages = [_checkpoint("SWITCHING_ACCOUNT")]

        config = Config(config_name=request.config_name)
        device = Device(config=config)
        account = AccountInfo(
            account=account_hint,
            account_alias="#".join(aliases),
            character=request.character_name or "",
            svr=request.server_name or "",
        )

        switcher = SwitchAccount(config=config, device=device, to=account)
        if not switcher.switchAccount():
            return FosterExecuteResponse(
                success=False,
                code="IDENTITY_MISMATCH",
                message="target account/character could not be selected",
                stages=stages,
            )

        stages.append(_checkpoint("VERIFYING_ACCOUNT"))

        detected_server = request.server_name
        try:
            detected_server = switcher.get_svr_name() or detected_server
        except Exception:
            pass

        detected = FosterDetectedIdentity(
            masked_account=account_hint,
            character_name=request.character_name,
            server_name=detected_server,
            game_uid=request.game_uid,
        )

        if request.server_name and detected_server and detected_server != request.server_name:
            return FosterExecuteResponse(
                success=False,
                code="IDENTITY_MISMATCH",
                message="selected server does not match trusted identity",
                stages=stages,
                detected_identity=detected,
            )

        stages.append(_checkpoint("RUNNING"))
        result = FosterOnceTask(config=config, device=device).execute_once()

        return FosterExecuteResponse(
            success=result.success,
            code=result.code,
            message=result.message,
            remaining_seconds=result.remaining_seconds,
            stages=stages,
            detected_identity=detected,
        )
