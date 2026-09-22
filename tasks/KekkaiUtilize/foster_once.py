from dataclasses import dataclass
from datetime import timedelta
from typing import Optional

from tasks.GameUi.page import page_guild
from tasks.KekkaiUtilize.script_task import ScriptTask


@dataclass
class FosterOnceResult:
    success: bool
    code: str
    message: str
    remaining_seconds: Optional[int] = None


class FosterOnceTask(ScriptTask):
    """Execute exactly one foster attempt without touching OAS scheduler timing."""

    def _remaining_seconds(self) -> Optional[int]:
        self.screenshot()
        remaining = self.O_UTILIZE_RES_TIME.ocr(self.device.image)
        if not isinstance(remaining, timedelta):
            return None
        seconds = int(remaining.total_seconds())
        return seconds if seconds > 0 else None

    def execute_once(self) -> FosterOnceResult:
        con = self.config.kekkai_utilize.utilize_config

        self.ui_get_current_page()
        self.ui_goto(page_guild)
        self.goto_realm()
        self.realm_goto_grown()
        self.screenshot()

        if not self.appear(self.I_UTILIZE_ADD):
            return FosterOnceResult(
                success=True,
                code="SUCCESS",
                message="foster is already active",
                remaining_seconds=self._remaining_seconds(),
            )

        if not self.grown_goto_utilize():
            return FosterOnceResult(
                success=False,
                code="GAME_BUSY",
                message="unable to enter foster friend list",
            )

        self.foster_bridge_error_code = None
        self.foster_bridge_error_message = None
        result = self.run_utilize(
            con.select_friend_list,
            con.shikigami_class,
            con.shikigami_order,
        )

        if self.foster_bridge_error_code:
            return FosterOnceResult(
                success=False,
                code=self.foster_bridge_error_code,
                message=self.foster_bridge_error_message or "foster attempt failed",
            )

        if not result:
            return FosterOnceResult(
                success=False,
                code="UNKNOWN",
                message="foster attempt returned no success result",
            )

        self.back_guild()
        self.goto_realm()
        self.realm_goto_grown()

        return FosterOnceResult(
            success=True,
            code="SUCCESS",
            message="foster completed",
            remaining_seconds=self._remaining_seconds(),
        )
