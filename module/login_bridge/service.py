import time
from typing import List, Optional

from module.config.config import Config
from module.device.device import Device
from tasks.Component.SwitchAccount.login_account import LoginAccount

from module.login_bridge.models import LoginDetectResponse


def _clean_text(value) -> Optional[str]:
    if not isinstance(value, str):
        return None
    value = value.strip()
    return value or None


def _clean_character(value: str) -> str:
    return value.lstrip("1234567890 ([<>])【】（）《》").strip()


class LoginDetectService:
    def detect(self, config_name: str) -> LoginDetectResponse:
        try:
            config = Config(config_name=config_name)
            device = Device(config=config)
            detector = LoginAccount(config=config, device=device)

            detector.screenshot()
            self._normalize_login_form(detector)
            detector.screenshot()

            if not detector.appear(detector.I_CHECK_LOGIN_FORM):
                return LoginDetectResponse(
                    ready=False,
                    message="waiting for game login form after QR scan",
                )

            masked_account = self._detect_account(detector)
            server_name = _clean_text(detector.get_svr_name())
            characters = self._detect_characters(detector)

            if len(characters) > 1:
                return LoginDetectResponse(
                    ready=False,
                    ambiguous=True,
                    message="multiple game characters detected; refusing automatic selection",
                    masked_account=masked_account,
                    server_name=server_name,
                )

            if len(characters) != 1 or not server_name:
                return LoginDetectResponse(
                    ready=False,
                    message="waiting for a unique character and server",
                    masked_account=masked_account,
                    server_name=server_name,
                )

            return LoginDetectResponse(
                ready=True,
                message="game identity detected",
                masked_account=masked_account,
                character_name=characters[0],
                server_name=server_name,
            )
        except Exception as error:
            return LoginDetectResponse(
                ready=False,
                message=f"login identity detection not ready: {error}",
            )

    def select_platform(self, config_name: str, platform: str) -> LoginDetectResponse:
        try:
            platform = (platform or "").strip().upper()
            if platform not in {"ANDROID", "IOS"}:
                return LoginDetectResponse(
                    ready=False,
                    message=f"unsupported login platform: {platform}",
                )

            config = Config(config_name=config_name)
            device = Device(config=config)
            detector = LoginAccount(config=config, device=device)

            for _ in range(60):
                detector.screenshot()

                if detector.appear(detector.I_SA_LOGIN_FORM_APPLE) or detector.appear(
                    detector.I_SA_LOGIN_FORM_ANDROID
                ):
                    target = (
                        detector.I_SA_LOGIN_FORM_ANDROID
                        if platform == "ANDROID"
                        else detector.I_SA_LOGIN_FORM_APPLE
                    )
                    detector.ui_click_until_disappear(target, interval=0.5)
                    return LoginDetectResponse(
                        ready=True,
                        message=f"{platform} login platform selected",
                    )

                if detector.appear(detector.I_CHECK_LOGIN_FORM):
                    return LoginDetectResponse(
                        ready=True,
                        message="login platform screen already completed",
                    )

                time.sleep(0.5)

            return LoginDetectResponse(
                ready=False,
                message="waiting for Android/iOS platform selection screen",
            )
        except Exception as error:
            return LoginDetectResponse(
                ready=False,
                message=f"login platform selection failed: {error}",
            )

    def select_identity(
        self,
        config_name: str,
        server_name: str,
        character_name: str,
    ) -> LoginDetectResponse:
        try:
            character_name = (character_name or "").strip()
            server_name = (server_name or "").strip()
            if not character_name:
                return LoginDetectResponse(
                    ready=False,
                    message="character name is required",
                )

            config = Config(config_name=config_name)
            device = Device(config=config)
            detector = LoginAccount(config=config, device=device)

            detector.screenshot()
            self._normalize_login_form(detector)
            detector.screenshot()

            if not detector.appear(detector.I_CHECK_LOGIN_FORM):
                return LoginDetectResponse(
                    ready=False,
                    message="waiting for game login form before character selection",
                )

            masked_account = self._detect_account(detector)
            detector.screenshot()

            if not detector.switch_character(character_name):
                return LoginDetectResponse(
                    ready=False,
                    message=f"target character not found: {character_name}",
                    masked_account=masked_account,
                )

            detector.screenshot()
            actual_server = _clean_text(detector.get_svr_name())
            if not actual_server:
                return LoginDetectResponse(
                    ready=False,
                    message="target character selected but server name is not ready",
                    masked_account=masked_account,
                    character_name=character_name,
                )

            message = "target character selected"
            if server_name and actual_server != server_name:
                message = (
                    f"target character selected but server mismatch: "
                    f"expected {server_name}, detected {actual_server}"
                )

            return LoginDetectResponse(
                ready=True,
                message=message,
                masked_account=masked_account,
                character_name=character_name,
                server_name=actual_server,
                game_uid=None,
            )
        except Exception as error:
            return LoginDetectResponse(
                ready=False,
                message=f"login identity selection failed: {error}",
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
