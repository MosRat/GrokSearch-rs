from setuptools import setup
from setuptools.command.bdist_wheel import bdist_wheel


class BDistWheel(bdist_wheel):
    def finalize_options(self):
        super().finalize_options()
        self.root_is_pure = False


setup(cmdclass={"bdist_wheel": BDistWheel})
