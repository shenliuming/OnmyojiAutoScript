import time
from typing import List, Optional

from module.config.config import Config
from module.device.device import Device
from tasks.Component.SwitchAccount.login_account import LoginAccount

from module.login_bridge.models import LoginDetectRequest, LoginDetectResponse


def _clean_text(value) -> Optional[str]:
    if not isinstance(value, str):
        return None
    value = value.strip()
    return value or None


def _clean_character(value: str) -> str:
    return value.lstrip("1234567890 ([<>])【】（）《》").strip()


class LoginDetectService:
    def detect(self, request: LoginDetectRequest) -> LoginDetectResponse:
        try:
            config = Config(config_name=request.config_name)
            device = Device(config=config)
            detector = LoginAccount(config=config, device=device)

            detector.screenshot()
            self._select_platform_if_needed(detector, request.platform)
            detector.screenshot()
            self._normalize_login_form(detector)
            detector.screenshot()

            if not detector.appear(detector.I_CHECK_LOGIN_FORM):
                return LoginDetectResponse(
                    ready=False,
                    message="waiting for game login form after QR scan",
                )

            masked_account = self._detect_account(detector)
            detector.screenshot()

            if not detector.switch_character(request.character_name):
                return LoginDetectResponse(
                    ready=False,
                    message=f"waiting for target character: {request.character_name}",
                    masked_account=masked_account,
                )

            detector.screenshot()
            server_name = _clean_text(detector.get_svr_name())
            if not server_name:
                return LoginDetectResponse(
                    ready=False,
                    message="target character selected but server name is not ready",
                    masked_account=masked_account,
                    character_name=request.character_name,
                )

            return LoginDetectResponse(
                ready=True,
                message="target game identity detected",
                masked_account=masked_account,
                character_name=request.character_name,
                server_name=server_name,
                game_uid=request.game_uid,
            )
        except Exception as error:
            return LoginDetectResponse(
                ready=False,
                message=f"login identity detection not ready: {error}",
            )

    def _normalize_login_form(self, detector: LoginAccount) -> None:
        if detector.appear(detector.I_SA_CHECK_SELECT_SVR_1) or detector.appear(
            detector.I_SA_CHECK_SELECT_SVR_2
        ):
            detector.click(detector.C_SA_LOGIN_FORM_CANCEL_SVR_SELECT, 0.5)
            time.sleep(0.5)
            detector.screenshot()

        if detector.appear(detector.I_SA_SWITCH_ACCOUNT_BTN):
            detector.click(detector.C_SA_LOGIN_FORM_USER_CENTER_CLOSE_BTN, 0.5)
            time.sleep(0.5)

    def _detect_account(self, detector: LoginAccount) -> Optional[str]:
        detector.screenshot()
        if not detector.appear(detector.I_CHECK_LOGIN_FORM):
            return None

        detector.click(detector.C_SA_LOGIN_FORM_USER_CENTER, 0.5)
        time.sleep(0.7)
        detector.screenshot()

        if not detector.appear(detector.I_SA_SWITCH_ACCOUNT_BTN):
            return None

        account = _clean_text(
            detector.O_SA_LOGIN_FORM_USER_CENTER_ACCOUNT.ocr_single(
                detector.device.image
            )
        )
        detector.click(detector.C_SA_LOGIN_FORM_USER_CENTER_CLOSE_BTN, 0.5)
        time.sleep(0.5)
        return account

    def _detect_characters(self, detector: LoginAccount) -> List[str]:
        detector.screenshot()
        if not detector.appear(detector.I_CHECK_LOGIN_FORM):
            return []

        detector.click(detector.C_SA_LOGIN_FORM_SWITCH_SVR_BTN, 0.7)
        time.sleep(0.8)
        detector.screenshot()

        if detector.appear(detector.I_SA_CHECK_SELECT_SVR_1) and not detector.appear(
            detector.I_SA_CHECK_SELECT_SVR_2
        ):
            detector.click(detector.C_SA_SELECT_SVR_CHARACTER_LIST, 0.7)
            time.sleep(0.8)
            detector.screenshot()

        if not detector.appear(detector.I_SA_CHECK_SELECT_SVR_2):
            detector.click(detector.C_SA_LOGIN_FORM_CANCEL_SVR_SELECT, 0.5)
            return []

        results = detector.O_SA_SELECT_SVR_CHARACTER_LIST.detect_and_ocr(
            detector.device.image
        )
        names = []
        for result in results:
            name = _clean_character(result.ocr_text)
            if name and name not in names:
                names.append(name)

        detector.click(detector.C_SA_LOGIN_FORM_CANCEL_SVR_SELECT, 0.5)
        time.sleep(0.5)
        return names
