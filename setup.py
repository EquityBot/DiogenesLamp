"""Copy the dependency-free viewer into wheels without a frontend build."""

from pathlib import Path
from shutil import copy2

from setuptools import setup
from setuptools.command.build_py import build_py


class BuildViewer(build_py):
    def run(self):
        super().run()
        target = Path(self.build_lib) / "lamp" / "viewer"
        target.mkdir(parents=True, exist_ok=True)
        for source in (Path(__file__).parent / "viewer").iterdir():
            if source.suffix in {".html", ".css", ".js", ".json"}:
                copy2(source, target / source.name)


setup(cmdclass={"build_py": BuildViewer})
