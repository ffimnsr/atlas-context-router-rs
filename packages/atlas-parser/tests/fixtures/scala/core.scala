package demo.app

import demo.support.Helper

object Runner {
  def helper(): Unit = ()
  def run(): Unit = helper()
}

case class Box(value: Int)

class Worker {
  def work(): Unit = ()
}

trait Service {
  def ping(): Unit
}
