import math
import os
from pathlib import Path
import unittest

from yuriengine import Scene


DEFAULT_AVATAR = Path(r"G:\Tsugu\sample\MUMUS_Avatar\fbx\Mumus_fbx.fbx")
FBX_PATH = Path(os.environ.get("YURIENGINE_TEST_FBX", str(DEFAULT_AVATAR)))


class FbxImportTests(unittest.TestCase):
    def test_loads_requested_avatar_fbx(self):
        if not FBX_PATH.exists():
            self.skipTest(f"set YURIENGINE_TEST_FBX to an available FBX file ({FBX_PATH})")

        scene = Scene()
        avatar = scene.load_fbx(FBX_PATH, name="Avatar")

        self.assertGreater(avatar.source_meshes, 0)
        self.assertGreater(avatar.triangles, 0)
        self.assertEqual(avatar.vertex_count, avatar.triangles * 3)
        self.assertTrue(
            all(math.isfinite(value) for position in avatar.vertex_positions for value in position)
        )
        bounds_min = avatar.bounds_min
        bounds_max = avatar.bounds_max
        self.assertGreater(bounds_max[0] - bounds_min[0], 0.01)
        self.assertGreater(bounds_max[1] - bounds_min[1], 0.01)
        self.assertGreater(bounds_max[2] - bounds_min[2], 0.01)


if __name__ == "__main__":
    unittest.main()
