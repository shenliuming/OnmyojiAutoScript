import unittest

from tasks.KekkaiUtilize.provider_target import (
    expected_resource_card_type,
    find_provider_card_area,
    normalize_provider_alias,
    provider_alias_matches,
)


class ProviderTargetTest(unittest.TestCase):
    def test_normalize_alias_removes_common_noise(self):
        self.assertEqual(normalize_provider_alias(" 资源-A·01 "), "资源a01")

    def test_alias_match_requires_meaningful_overlap(self):
        self.assertTrue(provider_alias_matches("资源A01", "资源 A01"))
        self.assertTrue(provider_alias_matches("ProviderOne", "ProviderOne Lv60"))
        self.assertTrue(provider_alias_matches("资源A01", "资源A01 等级60"))
        self.assertFalse(provider_alias_matches("A1", "A2"))
        self.assertFalse(provider_alias_matches("资源A", "资源B"))
        self.assertFalse(provider_alias_matches("资源A", "资源A备用"))
        self.assertFalse(provider_alias_matches("资源A", "VIP资源A"))


    def test_resource_type_maps_to_expected_game_card(self):
        self.assertEqual(expected_resource_card_type("FISH"), "斗鱼")
        self.assertEqual(expected_resource_card_type("DOUYU"), "斗鱼")
        self.assertEqual(expected_resource_card_type("TAIKO_JADE"), "太鼓")
        self.assertEqual(expected_resource_card_type("JADE"), "太鼓")
        self.assertIsNone(expected_resource_card_type("UNKNOWN"))

    def test_pair_name_with_nearest_card_row(self):
        ocr_entries = [
            ("资源A01", ((10, 15), (100, 15), (100, 45), (10, 45))),
            ("其他好友", ((10, 145), (100, 145), (100, 175), (10, 175))),
        ]
        card_areas = [
            (540, 108, 80, 60),
            (540, 238, 80, 60),
        ]

        matched = find_provider_card_area(
            "资源A01",
            ocr_entries,
            card_areas,
            ocr_origin_y=100,
        )

        self.assertEqual(matched, card_areas[0])

    def test_no_matching_provider_returns_none(self):
        matched = find_provider_card_area(
            "不存在",
            [("资源A01", ((0, 0), (20, 0), (20, 20), (0, 20)))],
            [(540, 100, 80, 60)],
            ocr_origin_y=100,
        )
        self.assertIsNone(matched)


if __name__ == "__main__":
    unittest.main()
