import re
from typing import Iterable, Optional, Sequence


def normalize_provider_alias(value: str) -> str:
    if not value:
        return ""
    value = value.strip().lower()
    return re.sub(r"[\s\-_.·•,，。:：;；()（）\[\]【】<>《》]+", "", value)


def provider_alias_matches(expected: str, observed: str) -> bool:
    expected_norm = normalize_provider_alias(expected)
    observed_norm = normalize_provider_alias(observed)
    if not expected_norm or not observed_norm:
        return False
    if expected_norm == observed_norm:
        return True
    shortest = min(len(expected_norm), len(observed_norm))
    return shortest >= 3 and (
        expected_norm in observed_norm or observed_norm in expected_norm
    )


def box_center_y(box: Sequence[Sequence[float]], origin_y: float = 0) -> float:
    ys = [point[1] for point in box]
    return origin_y + (min(ys) + max(ys)) / 2


def area_center_y(area: Sequence[float]) -> float:
    return float(area[1]) + float(area[3]) / 2


def find_provider_card_area(
    provider_alias: str,
    ocr_entries: Iterable[tuple[str, Sequence[Sequence[float]]]],
    card_areas: Iterable[Sequence[float]],
    *,
    ocr_origin_y: float = 0,
    max_y_distance: float = 85,
) -> Optional[Sequence[float]]:
    matching_name_centers = [
        box_center_y(box, ocr_origin_y)
        for text, box in ocr_entries
        if provider_alias_matches(provider_alias, text)
    ]
    if not matching_name_centers:
        return None

    best_area = None
    best_distance = None
    for area in card_areas:
        card_y = area_center_y(area)
        distance = min(abs(card_y - name_y) for name_y in matching_name_centers)
        if distance > max_y_distance:
            continue
        if best_distance is None or distance < best_distance:
            best_area = area
            best_distance = distance

    return best_area
